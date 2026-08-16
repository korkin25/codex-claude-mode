#!/usr/bin/env python3
"""Validate the public CCM capability manifest with Python stdlib only."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from typing import Any


CAPABILITY_RE = re.compile(r"^ccm\.[a-z0-9]+(?:[.-][a-z0-9]+)*\.v[1-9][0-9]*$")
WORK_ITEM_RE = re.compile(r"^CCM-[A-Z]+-[0-9]{3}$")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
RUN_RE = re.compile(r"^[0-9]+$")
SPEC_RE = re.compile(r"^(ROADMAP|TODO|ARCHITECTURE)\.md#[a-z0-9-]+$")
TODO_ROW_RE = re.compile(
    r"^\| `(?P<id>CCM-[A-Z]+-[0-9]{3})` \| `(?P<status>done|ready|planned|blocked)` "
    r"\| `(?P<lane>[a-z-]+)` \|.*\| (?P<dependencies>.*) \|$"
)

TOP_KEYS = {"document_type", "schema_version", "repository", "capabilities"}
CAPABILITY_KEYS = {
    "id", "work_item", "lane", "status", "optional", "dependencies",
    "specification", "policy_version", "content_scope", "required_checks",
    "blockers", "verification",
}
VERIFICATION_KEYS = {
    "merge_sha", "content_digest", "ci", "verified_at", "provenance",
}
CI_KEYS = {"provider", "run_id", "url", "head_sha", "status", "required_checks"}
LANES = {"core", "optional-daemon", "direct-compatibility"}
STATUSES = {"planned", "ready", "blocked", "verified"}
TODO_STATUS = {"planned": "planned", "ready": "ready", "blocked": "blocked", "verified": "done"}
REPOSITORY = "https://github.com/korkin25/codex-claude-mode"
V1_REQUIRED_CHECKS = [
    "Public capability manifest",
    "Rust 1.95 · linux-x86_64",
    "Rust 1.95 · macos-arm64",
]
SCHEMA_DIGEST = "sha256:814847fb1a68e7aa647198f446b6e06e5244407cf18542bbcf3f64d1789ffe9a"
IMMUTABLE_DECLARATION_FIELDS = {
    "id", "work_item", "lane", "optional", "dependencies", "specification",
    "policy_version", "content_scope", "required_checks",
}


class DuplicateKey(ValueError):
    pass


def _no_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKey(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json(path: Path, errors: list[str]) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_no_duplicate_keys)
    except (OSError, json.JSONDecodeError, DuplicateKey) as exc:
        errors.append(f"JSON_INVALID {path}: {exc}")
        return {}


def strict_keys(value: Any, expected: set[str], where: str, errors: list[str]) -> bool:
    if not isinstance(value, dict):
        errors.append(f"TYPE {where}: expected object")
        return False
    missing = sorted(expected - set(value))
    extra = sorted(set(value) - expected)
    if missing:
        errors.append(f"MISSING {where}: {','.join(missing)}")
    if extra:
        errors.append(f"UNKNOWN {where}: {','.join(extra)}")
    return not missing and not extra


def string_list(value: Any, where: str, errors: list[str]) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        errors.append(f"TYPE {where}: expected string array")
        return []
    if len(value) != len(set(value)):
        errors.append(f"DUPLICATE {where}")
    return value


def timestamp(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return False
    return parsed.tzinfo is not None


def canonical_tree_digest(root: Path, merge_sha: str, paths: list[str], where: str, errors: list[str]) -> str | None:
    commit = subprocess.run(
        ["git", "cat-file", "-e", f"{merge_sha}^{{commit}}"], cwd=root,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
    )
    if commit.returncode != 0:
        errors.append(f"GIT_COMMIT_UNKNOWN {where}.merge_sha")
        return None
    ancestor = subprocess.run(
        ["git", "merge-base", "--is-ancestor", merge_sha, "HEAD"], cwd=root,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
    )
    if ancestor.returncode != 0:
        errors.append(f"GIT_COMMIT_NOT_ANCESTOR {where}.merge_sha")
    records: set[bytes] = set()
    for path in paths:
        listed = subprocess.run(
            ["git", "ls-tree", "-rz", "--full-tree", merge_sha, "--", path],
            cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            check=False,
        )
        if listed.returncode != 0:
            errors.append(f"GIT_TREE_UNREADABLE {where}.content_scope")
            return None
        path_records = [record for record in listed.stdout.split(b"\0") if record]
        if not path_records:
            errors.append(f"GIT_TREE_EMPTY {where}.content_scope: {path}")
        records.update(path_records)
    records = sorted(records)
    if not records:
        errors.append(f"GIT_TREE_EMPTY {where}.content_scope")
        return None
    canonical = b"".join(record + b"\0" for record in records)
    return "sha256:" + hashlib.sha256(canonical).hexdigest()


def validate_verification(
    value: Any,
    content_scope: list[str],
    required_checks: list[str],
    root: Path,
    where: str,
    errors: list[str],
) -> None:
    if not strict_keys(value, VERIFICATION_KEYS, where, errors):
        return
    merge_sha = value.get("merge_sha")
    if not isinstance(merge_sha, str) or not SHA_RE.fullmatch(merge_sha):
        errors.append(f"FORMAT {where}.merge_sha")
    digest = value.get("content_digest")
    if not isinstance(digest, str) or not DIGEST_RE.fullmatch(digest):
        errors.append(f"FORMAT {where}.content_digest")
    if value.get("provenance") != "measured":
        errors.append(f"UNTRUSTED {where}.provenance")
    if not timestamp(value.get("verified_at")):
        errors.append(f"FORMAT {where}.verified_at")
    if isinstance(merge_sha, str) and SHA_RE.fullmatch(merge_sha) and content_scope:
        measured = canonical_tree_digest(root, merge_sha, content_scope, where, errors)
        if measured is not None and measured != digest:
            errors.append(f"DIGEST_MISMATCH {where}.content_digest")

    ci = value.get("ci")
    if not strict_keys(ci, CI_KEYS, f"{where}.ci", errors):
        return
    if ci.get("provider") != "github-actions" or ci.get("status") != "success":
        errors.append(f"CI_NOT_SUCCESS {where}.ci")
    run_id = ci.get("run_id")
    if not isinstance(run_id, str) or not RUN_RE.fullmatch(run_id):
        errors.append(f"FORMAT {where}.ci.run_id")
    expected_url = f"{REPOSITORY}/actions/runs/{run_id}"
    if ci.get("url") != expected_url:
        errors.append(f"FORMAT {where}.ci.url")
    if ci.get("head_sha") != merge_sha:
        errors.append(f"SHA_MISMATCH {where}.ci.head_sha")
    checks = string_list(ci.get("required_checks"), f"{where}.ci.required_checks", errors)
    if not checks or any(not check.strip() for check in checks):
        errors.append(f"EMPTY {where}.ci.required_checks")
    if checks != required_checks:
        errors.append(f"REQUIRED_CHECKS_MISMATCH {where}.ci.required_checks")


def validate_schema_contract(schema: Any, errors: list[str]) -> None:
    if not isinstance(schema, dict) or schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        errors.append("SCHEMA_INVALID capabilities.schema.json")
        return
    canonical = json.dumps(schema, sort_keys=True, separators=(",", ":")).encode("utf-8")
    actual_digest = "sha256:" + hashlib.sha256(canonical).hexdigest()
    if actual_digest != SCHEMA_DIGEST:
        errors.append("SCHEMA_CONTENT_DRIFT capabilities.schema.json")
    definitions = schema.get("$defs")
    if not isinstance(definitions, dict):
        errors.append("SCHEMA_DEFINITIONS_MISSING")
        return
    expected = {
        "capability": (CAPABILITY_KEYS, "status", STATUSES),
        "verification": (VERIFICATION_KEYS, None, set()),
        "ci": (CI_KEYS, None, set()),
    }
    for name, (keys, enum_field, enum_values) in expected.items():
        definition = definitions.get(name)
        if not isinstance(definition, dict):
            errors.append(f"SCHEMA_DEFINITION_MISSING {name}")
            continue
        if set(definition.get("required", [])) != keys:
            errors.append(f"SCHEMA_REQUIRED_DRIFT {name}")
        properties = definition.get("properties")
        if not isinstance(properties, dict) or set(properties) != keys:
            errors.append(f"SCHEMA_PROPERTIES_DRIFT {name}")
            continue
        if enum_field is not None and set(properties.get(enum_field, {}).get("enum", [])) != enum_values:
            errors.append(f"SCHEMA_ENUM_DRIFT {name}.{enum_field}")


def markdown_has_anchor(root: Path, specification: str) -> bool:
    filename, anchor = specification.split("#", 1)
    try:
        lines = (root / filename).read_text(encoding="utf-8").splitlines()
    except OSError:
        return False
    anchors: set[str] = set()
    for line in lines:
        match = re.match(r"^#{1,6}\s+(.+?)\s*$", line)
        if not match:
            continue
        heading = match.group(1).strip().lower()
        heading = re.sub(r"[^a-z0-9 -]", "", heading)
        anchors.add(re.sub(r"[ -]+", "-", heading).strip("-"))
    return anchor in anchors


def baseline_manifest(root: Path) -> dict[str, dict[str, Any]]:
    dirty = subprocess.run(
        ["git", "diff", "--quiet", "HEAD", "--", "delivery/capabilities.json"],
        cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
    ).returncode != 0
    revision = "HEAD" if dirty else "HEAD^"
    completed = subprocess.run(
        ["git", "show", f"{revision}:delivery/capabilities.json"], cwd=root,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False,
    )
    if completed.returncode != 0:
        return {}
    try:
        data = json.loads(completed.stdout, object_pairs_hook=_no_duplicate_keys)
    except (json.JSONDecodeError, DuplicateKey):
        return {}
    return {
        item.get("id"): item for item in data.get("capabilities", [])
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }


def todo_entries(path: Path, errors: list[str]) -> dict[str, tuple[str, str, list[str]]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        errors.append(f"TODO_INVALID {path}: {exc}")
        return {}
    entries: dict[str, tuple[str, str, list[str]]] = {}
    for line in lines:
        match = TODO_ROW_RE.match(line)
        if not match:
            continue
        work_item = match.group("id")
        if work_item in entries:
            errors.append(f"DUPLICATE TODO.work_item: {work_item}")
        dependencies = re.findall(r"CCM-[A-Z]+-[0-9]{3}", match.group("dependencies"))
        entries[work_item] = (match.group("status"), match.group("lane"), dependencies)
    return entries


def cycle_errors(capabilities: dict[str, dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(capability_id: str, trail: list[str]) -> None:
        if capability_id in visiting:
            start = trail.index(capability_id)
            errors.append("DEPENDENCY_CYCLE " + " -> ".join(trail[start:] + [capability_id]))
            return
        if capability_id in visited:
            return
        visiting.add(capability_id)
        trail.append(capability_id)
        for dependency in capabilities[capability_id].get("dependencies", []):
            if dependency in capabilities:
                visit(dependency, trail)
        trail.pop()
        visiting.remove(capability_id)
        visited.add(capability_id)

    for capability_id in capabilities:
        visit(capability_id, [])
    return errors


def validate_repository(root: Path) -> list[str]:
    errors: list[str] = []
    delivery = root / "delivery"
    schema = load_json(delivery / "capabilities.schema.json", errors)
    validate_schema_contract(schema, errors)

    manifest = load_json(delivery / "capabilities.json", errors)
    if not isinstance(manifest, dict):
        errors.append("TYPE manifest: expected object")
        return sorted(set(errors))
    strict_keys(manifest, TOP_KEYS, "manifest", errors)
    if manifest.get("document_type") != "ccm-capability-manifest":
        errors.append("DOCUMENT_TYPE manifest")
    if manifest.get("schema_version") != 1:
        errors.append("SCHEMA_VERSION manifest")
    if manifest.get("repository") != REPOSITORY:
        errors.append("REPOSITORY manifest")

    raw_capabilities = manifest.get("capabilities")
    if not isinstance(raw_capabilities, list) or not raw_capabilities:
        errors.append("TYPE manifest.capabilities: expected non-empty array")
        raw_capabilities = []
    capabilities: dict[str, dict[str, Any]] = {}
    work_items: set[str] = set()
    for index, capability in enumerate(raw_capabilities):
        where = f"capabilities[{index}]"
        if not strict_keys(capability, CAPABILITY_KEYS, where, errors):
            continue
        capability_id = capability.get("id")
        if not isinstance(capability_id, str) or not CAPABILITY_RE.fullmatch(capability_id):
            errors.append(f"FORMAT {where}.id")
            continue
        if capability_id in capabilities:
            errors.append(f"DUPLICATE capability.id: {capability_id}")
        capabilities[capability_id] = capability
        work_item = capability.get("work_item")
        if not isinstance(work_item, str) or not WORK_ITEM_RE.fullmatch(work_item):
            errors.append(f"FORMAT {where}.work_item")
        elif work_item in work_items:
            errors.append(f"DUPLICATE capability.work_item: {work_item}")
        else:
            work_items.add(work_item)
        if capability.get("lane") not in LANES:
            errors.append(f"VALUE {where}.lane")
        if not isinstance(capability.get("optional"), bool):
            errors.append(f"TYPE {where}.optional")
        if (capability.get("lane") == "optional-daemon") != (capability.get("optional") is True):
            errors.append(f"OPTIONAL_LANE_MISMATCH {where}")
        status = capability.get("status")
        if status not in STATUSES:
            errors.append(f"VALUE {where}.status")
        spec = capability.get("specification")
        if not isinstance(spec, str) or not SPEC_RE.fullmatch(spec):
            errors.append(f"FORMAT {where}.specification")
        elif not markdown_has_anchor(root, spec):
            errors.append(f"SPECIFICATION_NOT_FOUND {where}.specification")
        dependencies = string_list(capability.get("dependencies"), f"{where}.dependencies", errors)
        if capability_id in dependencies:
            errors.append(f"SELF_DEPENDENCY {capability_id}")
        if capability.get("policy_version") != 1:
            errors.append(f"POLICY_VERSION {where}.policy_version")
        content_scope = string_list(capability.get("content_scope"), f"{where}.content_scope", errors)
        if not content_scope:
            errors.append(f"EMPTY {where}.content_scope")
        safe_scope = [
            path for path in content_scope
            if path and not path.startswith("/") and ".." not in Path(path).parts
        ]
        if len(safe_scope) != len(content_scope):
            errors.append(f"UNSAFE {where}.content_scope")
        required_checks = string_list(capability.get("required_checks"), f"{where}.required_checks", errors)
        if not required_checks or any(not check.strip() for check in required_checks):
            errors.append(f"EMPTY {where}.required_checks")
        if capability.get("policy_version") == 1 and required_checks != V1_REQUIRED_CHECKS:
            errors.append(f"POLICY_CHECKS_MISMATCH {where}.required_checks")
        blockers = string_list(capability.get("blockers"), f"{where}.blockers", errors)
        if status == "blocked" and not blockers:
            errors.append(f"BLOCKER_MISSING {where}.blockers")
        if status != "blocked" and blockers:
            errors.append(f"UNEXPECTED_BLOCKER {where}.blockers")
        verification = capability.get("verification")
        if status == "verified":
            validate_verification(
                verification, safe_scope, required_checks, root,
                f"{where}.verification", errors,
            )
        elif verification is not None:
            errors.append(f"PREMATURE_EVIDENCE {where}.verification")

    baseline = baseline_manifest(root)
    for capability_id, capability in capabilities.items():
        if capability.get("status") == "verified":
            if capability_id not in baseline:
                errors.append(f"VERIFIED_WITHOUT_BASELINE {capability_id}")
            else:
                previous = baseline[capability_id]
                for field in sorted(IMMUTABLE_DECLARATION_FIELDS):
                    if capability.get(field) != previous.get(field):
                        errors.append(f"DECLARATION_CHANGED_DURING_SETTLEMENT {capability_id}.{field}")
        for dependency in capability.get("dependencies", []):
            if dependency not in capabilities:
                errors.append(f"UNKNOWN_DEPENDENCY {capability_id}: {dependency}")
        if capability.get("status") in {"ready", "verified"}:
            unfinished = [
                dependency for dependency in capability.get("dependencies", [])
                if capabilities.get(dependency, {}).get("status") != "verified"
            ]
            if unfinished:
                errors.append(f"DEPENDENCIES_NOT_VERIFIED {capability_id}: {','.join(unfinished)}")
    errors.extend(cycle_errors(capabilities))

    todos = todo_entries(root / "TODO.md", errors)
    for capability_id, capability in capabilities.items():
        work_item = capability.get("work_item")
        expected = todos.get(work_item)
        if expected is None:
            errors.append(f"TODO_MISSING {capability_id}: {work_item}")
            continue
        expected_status, expected_lane, expected_dependencies = expected
        if expected_status != TODO_STATUS.get(capability.get("status")):
            errors.append(f"TODO_STATUS_MISMATCH {work_item}")
        if expected_lane != capability.get("lane"):
            errors.append(f"TODO_LANE_MISMATCH {work_item}")
        actual_dependencies = [
            capabilities[dependency].get("work_item")
            for dependency in capability.get("dependencies", [])
            if dependency in capabilities
        ]
        if expected_dependencies != actual_dependencies:
            errors.append(f"TODO_DEPENDENCY_MISMATCH {work_item}")
    return sorted(set(errors))


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    errors = validate_repository(root)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        print(f"capability manifest validation: FAILED ({len(errors)} errors)", file=sys.stderr)
        return 1
    count = len(load_json(root / "delivery" / "capabilities.json", [])["capabilities"])
    print(f"capability manifest validation: OK capabilities={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
