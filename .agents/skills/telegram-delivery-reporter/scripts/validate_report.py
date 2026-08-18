#!/usr/bin/env python3
"""Fail-closed validator and renderer for Telegram merge reports."""

from __future__ import annotations

import html
import json
import posixpath
import re
import sys
from typing import Any


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
HTTPS_RE = re.compile(r"^https://[^\s]+$")
BRANCH_RE = re.compile(r"^(?![./])(?!.*(?:\.\.|//|@\{|\\))[A-Za-z0-9][A-Za-z0-9._/-]{0,199}(?<![./])$")
KNOWN_CI_STATES = {
    "pending", "queued", "in_progress", "success", "failure", "cancelled",
    "timed_out", "action_required", "skipped", "neutral", "stale",
}
MAX_LIST_ITEMS = 64
MAX_MESSAGE_CHARS = 4096
SHORT_SHA_CHARS = 8
# Canonical "checked and empty" answers. Only these collapse an optional
# section; any other text is a real limitation and stays visible.
NO_CONTENT_MARKERS = frozenset({"нет", "нет известных", "отсутствуют"})
EVENT_KEYS = {
    "postmerge": {
        "event", "repository", "change", "reason", "components",
        "security_authority_impact", "tests", "limitations", "blockers",
        "unverified", "source_branch", "target_branch", "candidate_sha",
        "merge_sha", "commit_url", "pr_url", "ci", "files",
    },
    "correction": {
        "event", "prior_message_id", "repository", "context_sha", "field_type",
        "field", "old_value", "replacement", "unchanged_or_corrected",
        "evidence", "unverified",
    },
}
EVIDENCE_LINK_LABELS = {
    "github_commit": "commit", "github_pull_request": "PR", "github_actions_run": "CI",
}
CORRECTION_FIELDS = {
    "branch": {"source_branch", "target_branch"},
    "url": {"commit_url", "pr_url", "ci.url"},
    "ci_state": {"ci.state"},
}


class ReportError(ValueError):
    pass


class _VerifiedReport:
    """Process-local capability created only after provider/ref verification."""

    __slots__ = ("payload", "checks")

    def __init__(self, payload: dict[str, Any], checks: tuple[str, ...]) -> None:
        self.payload = payload
        self.checks = checks


def _verified_report(payload: dict[str, Any], checks: list[str]) -> _VerifiedReport:
    if not checks:
        raise ReportError("verification: no trusted checks")
    return _VerifiedReport(payload, tuple(checks))


def strict_keys(value: Any, expected: set[str], where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ReportError(f"{where}: expected object")
    missing = sorted(expected - set(value))
    extra = sorted(set(value) - expected)
    if missing:
        raise ReportError(f"{where}: missing {','.join(missing)}")
    if extra:
        raise ReportError(f"{where}: unknown {','.join(extra)}")
    return value


def one_line(value: Any, where: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        raise ReportError(f"{where}: expected non-empty trimmed string")
    if "\n" in value or "\r" in value or len(value) > 1000:
        raise ReportError(f"{where}: multiline or oversized value")
    return value


def string_list(value: Any, where: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ReportError(f"{where}: expected non-empty array")
    if len(value) > MAX_LIST_ITEMS:
        raise ReportError(f"{where}: too many values")
    result = [one_line(item, f"{where}[{index}]") for index, item in enumerate(value)]
    if len(result) != len(set(result)):
        raise ReportError(f"{where}: duplicate value")
    return result


def changed_file(value: Any, where: str) -> str:
    value = one_line(value, where)
    if (
        value.startswith("/")
        or value == "."
        or posixpath.normpath(value) != value
        or any(component in {"", ".", ".."} for component in value.split("/"))
        or any(ord(character) < 32 or ord(character) == 127 for character in value)
    ):
        raise ReportError(f"{where}: expected normalized repository-relative path")
    return value


def changed_files(value: Any) -> list[str]:
    values = string_list(value, "files")
    return [changed_file(item, f"files[{index}]") for index, item in enumerate(values)]


def escaped(value: str) -> str:
    return html.escape(value, quote=True)


def link(label: str, href: str) -> str:
    return f'<a href="{escaped(href)}">{escaped(label)}</a>'


def telegram_length(value: str) -> int:
    """Count UTF-16 code units, the unit Telegram bounds `sendMessage` text by."""
    return len(value.encode("utf-16-le")) // 2


def reported(value: str) -> bool:
    """Report a field only when it is not a canonical checked-and-empty answer."""
    return value.casefold().rstrip(" .") not in NO_CONTENT_MARKERS


def short_sha(value: str) -> str:
    return value[:SHORT_SHA_CHARS]


def sha(value: Any, where: str) -> str:
    value = one_line(value, where)
    if not SHA_RE.fullmatch(value):
        raise ReportError(f"{where}: expected 40-hex SHA")
    return value


def repository(value: Any) -> str:
    value = one_line(value, "repository")
    if not REPOSITORY_RE.fullmatch(value):
        raise ReportError("repository: expected owner/name")
    return value


def branch(value: Any, where: str) -> str:
    value = one_line(value, where)
    if not BRANCH_RE.fullmatch(value):
        raise ReportError(f"{where}: expected canonical branch")
    components = value.split("/")
    if any(component.startswith(".") or component.endswith(".lock") for component in components):
        raise ReportError(f"{where}: expected canonical branch")
    return value


def url(value: Any, where: str) -> str:
    value = one_line(value, where)
    if not HTTPS_RE.fullmatch(value):
        raise ReportError(f"{where}: expected HTTPS URL")
    return value


def exact_github_url(value: Any, repository_id: str, suffix: str, where: str) -> str:
    value = url(value, where)
    expected = f"https://github.com/{repository_id}/{suffix}"
    if value != expected:
        raise ReportError(f"{where}: identity mismatch")
    return value


def github_commit_url(value: Any, repository_id: str, commit_sha: str, where: str) -> str:
    return exact_github_url(value, repository_id, f"commit/{commit_sha}", where)


def github_repository_url(repository_id: str) -> str:
    """Build the canonical project URL from the verifier-bound repository ID."""
    return f"https://github.com/{repository(repository_id)}"


def github_pr_url(value: Any, repository_id: str, where: str) -> str:
    value = url(value, where)
    if not re.fullmatch(rf"https://github\.com/{re.escape(repository_id)}/pull/[1-9][0-9]*", value):
        raise ReportError(f"{where}: repository or PR mismatch")
    return value


def github_pr(value: Any, repository_id: str, where: str) -> tuple[str, int]:
    """Return the verified PR URL together with its own PR number."""
    value = github_pr_url(value, repository_id, where)
    return value, int(value.rsplit("/", 1)[-1])


def github_run_url(value: Any, repository_id: str, where: str) -> str:
    value = url(value, where)
    if not re.fullmatch(rf"https://github\.com/{re.escape(repository_id)}/actions/runs/[1-9][0-9]*", value):
        raise ReportError(f"{where}: repository or run mismatch")
    return value


def successful_ci(value: Any, repository_id: str, expected_sha: str) -> tuple[str, str]:
    value = strict_keys(value, {"state", "head_sha", "url"}, "ci")
    state = one_line(value["state"], "ci.state")
    if state != "success":
        raise ReportError("ci.state: successful merge-SHA CI required")
    if sha(value["head_sha"], "ci.head_sha") != expected_sha:
        raise ReportError("ci.head_sha: exact-SHA mismatch")
    return state, github_run_url(value["url"], repository_id, "ci.url")


def correction_evidence(value: Any, repository_id: str, context_sha: str) -> tuple[str, str]:
    value = strict_keys(value, {"source", "url", "head_sha", "ref"}, "evidence")
    source = one_line(value["source"], "evidence.source")
    branch(value["ref"], "evidence.ref")
    if sha(value["head_sha"], "evidence.head_sha") != context_sha:
        raise ReportError("evidence.head_sha: context-SHA mismatch")
    if source == "github_commit":
        evidence_url = github_commit_url(value["url"], repository_id, context_sha, "evidence.url")
    elif source == "github_pull_request":
        evidence_url = github_pr_url(value["url"], repository_id, "evidence.url")
    elif source == "github_actions_run":
        evidence_url = github_run_url(value["url"], repository_id, "evidence.url")
    else:
        raise ReportError("evidence.source: unsupported source")
    return source, evidence_url


def _render_payload(data: Any) -> str:
    if not isinstance(data, dict):
        raise ReportError("report: expected object")
    event = data.get("event")
    if event not in EVENT_KEYS:
        raise ReportError("event: unsupported")
    data = strict_keys(data, EVENT_KEYS[event], "report")

    if event == "postmerge":
        repository_id = repository(data["repository"])
        change = one_line(data["change"], "change")
        reason = one_line(data["reason"], "reason")
        components = string_list(data["components"], "components")
        files = changed_files(data["files"])
        tests = string_list(data["tests"], "tests")
        details = {
            field: one_line(data[field], field)
            for field in ("security_authority_impact", "limitations", "blockers", "unverified")
        }
        source = branch(data["source_branch"], "source_branch")
        target = branch(data["target_branch"], "target_branch")
        sha(data["candidate_sha"], "candidate_sha")
        merge_sha = sha(data["merge_sha"], "merge_sha")
        project = github_repository_url(repository_id)
        commit = github_commit_url(data["commit_url"], repository_id, merge_sha, "commit_url")
        pull, pr_number = github_pr(data["pr_url"], repository_id, "pr_url")
        ci_state, ci_url = successful_ci(data["ci"], repository_id, merge_sha)
        lines = [
            f"✅ <b>{escaped(repository_id)}: merge завершён</b>",
            escaped(change),
            "",
            "🎯 <b>Зачем</b>",
            escaped(reason),
            "",
            "🧩 <b>Что изменилось</b>",
            *(f"• {escaped(item)}" for item in components),
            "",
            "📄 <b>Изменённые файлы</b>",
            *(f"• <code>{escaped(item)}</code>" for item in files),
            "",
            "🧪 <b>Проверено</b>",
            (f"• PR #{pr_number}: merged, <code>{escaped(source)}</code> → "
             f"<code>{escaped(target)}</code>"),
            *(f"• {escaped(item)}" for item in tests),
            "",
            "🔐 <b>Безопасность и полномочия</b>",
            f"• {escaped(details['security_authority_impact'])}",
        ]
        caveats = [
            f"• {label}: {escaped(details[field])}"
            for label, field in (
                ("Ограничения", "limitations"),
                ("Блокеры", "blockers"),
                ("Непроверенное", "unverified"),
            )
            if reported(details[field])
        ]
        if caveats:
            lines += ["", "⚠️ <b>Ограничения</b>", *caveats]
        lines += [
            "",
            "🔗 <b>Ссылки</b>",
            " · ".join((
                link(repository_id, project),
                link(f"commit {short_sha(merge_sha)}", commit),
                link(f"PR #{pr_number}", pull),
                link(f"CI {ci_state}", ci_url),
            )),
        ]
        return "\n".join(lines)

    message_id = data["prior_message_id"]
    if not isinstance(message_id, int) or isinstance(message_id, bool) or message_id < 1:
        raise ReportError("prior_message_id: expected positive merge-report receipt")
    repository_id = repository(data["repository"])
    context_sha = sha(data["context_sha"], "context_sha")
    field_type = one_line(data["field_type"], "field_type")
    field = one_line(data["field"], "field")
    if field_type not in CORRECTION_FIELDS or field not in CORRECTION_FIELDS[field_type]:
        raise ReportError("correction field/type is not a merge-report field")
    old_value = one_line(data["old_value"], "old_value")
    replacement = one_line(data["replacement"], "replacement")
    if old_value == replacement:
        raise ReportError("replacement: must differ from old_value")
    evidence_source, evidence_url = correction_evidence(data["evidence"], repository_id, context_sha)
    if field_type == "branch":
        branch(old_value, "old_value")
        branch(replacement, "replacement")
        if evidence_source != "github_pull_request":
            raise ReportError("evidence.source: merge branch correction requires github_pull_request")
    elif field_type == "url":
        if field == "commit_url":
            github_commit_url(replacement, repository_id, context_sha, "replacement")
            if evidence_source != "github_commit" or evidence_url != replacement:
                raise ReportError("evidence: merge commit URL correction source mismatch")
        elif field == "pr_url":
            github_pr_url(old_value, repository_id, "old_value")
            github_pr_url(replacement, repository_id, "replacement")
            if evidence_source != "github_pull_request" or evidence_url != replacement:
                raise ReportError("evidence: merged PR URL correction source mismatch")
        else:
            github_run_url(old_value, repository_id, "old_value")
            github_run_url(replacement, repository_id, "replacement")
            if evidence_source != "github_actions_run" or evidence_url != replacement:
                raise ReportError("evidence: merge CI URL correction source mismatch")
    else:
        if old_value not in KNOWN_CI_STATES or replacement != "success":
            raise ReportError("replacement: merge CI correction must be success")
        if evidence_source != "github_actions_run":
            raise ReportError("evidence.source: merge CI correction requires github_actions_run")
    unchanged = one_line(data["unchanged_or_corrected"], "unchanged_or_corrected")
    unverified = one_line(data["unverified"], "unverified")
    project = github_repository_url(repository_id)
    lines = [
        f"✏️ <b>{escaped(repository_id)}: исправление merge-отчёта</b>",
        (f"Сообщение <code>{message_id}</code>, merge "
         f"<code>{escaped(context_sha)}</code>."),
        "",
        "🔧 <b>Исправленное поле</b>",
        f"• Поле: <code>{escaped(field)}</code> ({escaped(field_type)})",
        f"• Было: {escaped(old_value)}",
        f"• Верно: {escaped(replacement)}",
        f"• Источник: {escaped(evidence_source)}",
        "",
        "🧪 <b>Остальные факты</b>",
        f"• {escaped(unchanged)}",
    ]
    if reported(unverified):
        lines += ["", "⚠️ <b>Ограничения</b>", f"• Непроверенное: {escaped(unverified)}"]
    lines += [
        "",
        "🔗 <b>Ссылки</b>",
        " · ".join((
            link(repository_id, project),
            link(EVIDENCE_LINK_LABELS[evidence_source], evidence_url),
        )),
    ]
    return "\n".join(lines)


def render(report: Any) -> str:
    if not isinstance(report, _VerifiedReport):
        raise ReportError("report: trusted provider/ref verification required")
    rendered = _render_payload(report.payload)
    if telegram_length(rendered) > MAX_MESSAGE_CHARS:
        raise ReportError("report: Telegram message exceeds 4096 characters")
    return rendered


def main() -> int:
    print(json.dumps({"valid": False, "errors": [
        "direct rendering forbidden; use verify_report.py"
    ]}, ensure_ascii=False, sort_keys=True), file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
