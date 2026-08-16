#!/usr/bin/env python3
"""Validate and inspect a restart-safe CCM task-branch checkpoint."""

from __future__ import annotations

import argparse
from datetime import datetime
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
WORK_RE = re.compile(r"^CCM-[A-Z]+-[0-9]{3}$")
CAPABILITY_RE = re.compile(r"^ccm\.[a-z0-9]+(?:[.-][a-z0-9]+)*\.v[1-9][0-9]*$")
REMOTE = "git@github.com:korkin25/codex-claude-mode.git"
TOP_KEYS = {
    "document_type", "schema_version", "repository", "admission", "execution", "checkpoint"
}
REPOSITORY_KEYS = {"id", "remote", "default_branch"}
ADMISSION_KEYS = {
    "controller_commit_sha", "claim_digest", "claim_id", "claim_generation",
    "owner_principal", "work_item_id", "capability_id", "base_sha", "branch",
    "issued_at", "expires_at", "dependency_evidence_refs",
}
EXECUTION_KEYS = {"phase", "completed_acceptance", "remaining_work", "completed_checks"}
CHECKPOINT_KEYS = {"sequence", "kind", "parent_sha", "updated_at", "next_action"}
CHECK_KEYS = {"command", "outcome"}
PHASES = {"claimed", "implementing", "checkpointed", "candidate", "blocked"}
KINDS = {"claim", "progress", "planned_restart", "candidate", "blocked"}


class DuplicateKey(ValueError):
    pass


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKey(key)
        result[key] = value
    return result


def load_state(path: Path, errors: list[str]) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
    except FileNotFoundError:
        errors.append(f"STATE_NOT_FOUND {path}")
        return {}
    except DuplicateKey as exc:
        errors.append(f"DUPLICATE_KEY {exc}")
        return {}
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        errors.append(f"STATE_UNREADABLE {exc}")
        return {}
    if not isinstance(value, dict):
        errors.append("TYPE state: expected object")
        return {}
    return value


def strict_object(value: Any, keys: set[str], where: str, errors: list[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        errors.append(f"TYPE {where}: expected object")
        return {}
    missing = sorted(keys - set(value))
    extra = sorted(set(value) - keys)
    if missing:
        errors.append(f"MISSING {where}: {','.join(missing)}")
    if extra:
        errors.append(f"UNKNOWN {where}: {','.join(extra)}")
    return value


def text(value: Any, where: str, errors: list[str], pattern: re.Pattern[str] | None = None) -> str | None:
    if not isinstance(value, str) or not value or (pattern is not None and not pattern.fullmatch(value)):
        errors.append(f"FORMAT {where}")
        return None
    return value


def timestamp(value: Any, where: str, errors: list[str]) -> datetime | None:
    if not isinstance(value, str):
        errors.append(f"FORMAT {where}")
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        errors.append(f"FORMAT {where}")
        return None
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        errors.append(f"TIMEZONE_REQUIRED {where}")
        return None
    return parsed


def string_list(value: Any, where: str, errors: list[str], *, allow_empty: bool = True) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        errors.append(f"TYPE {where}: expected {'non-empty ' if not allow_empty else ''}array")
        return []
    if not all(isinstance(item, str) and item for item in value):
        errors.append(f"FORMAT {where}")
        return []
    if len(value) != len(set(value)):
        errors.append(f"DUPLICATE {where}")
    return value


def valid_branch(branch: str | None) -> bool:
    return bool(
        branch
        and not branch.startswith(('.', '/'))
        and not branch.endswith(('.', '/'))
        and '..' not in branch
        and '@{' not in branch
        and not re.search(r"[\x00-\x20~^:?*\\]", branch)
    )


def validate_state(state: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    strict_object(state, TOP_KEYS, "state", errors)
    if state.get("document_type") != "ccm-delivery-execution-state":
        errors.append("DOCUMENT_TYPE")
    if state.get("schema_version") != 1:
        errors.append("SCHEMA_VERSION")

    repository = strict_object(state.get("repository"), REPOSITORY_KEYS, "repository", errors)
    if repository.get("id") != "ccm-public":
        errors.append("REPOSITORY_ID")
    if repository.get("remote") != REMOTE:
        errors.append("REPOSITORY_REMOTE")
    if repository.get("default_branch") != "main":
        errors.append("DEFAULT_BRANCH")

    admission = strict_object(state.get("admission"), ADMISSION_KEYS, "admission", errors)
    text(admission.get("controller_commit_sha"), "admission.controller_commit_sha", errors, SHA_RE)
    text(admission.get("claim_digest"), "admission.claim_digest", errors, DIGEST_RE)
    text(admission.get("claim_id"), "admission.claim_id", errors, ID_RE)
    generation = admission.get("claim_generation")
    if not isinstance(generation, int) or isinstance(generation, bool) or generation < 1:
        errors.append("FORMAT admission.claim_generation")
    text(admission.get("owner_principal"), "admission.owner_principal", errors, ID_RE)
    text(admission.get("work_item_id"), "admission.work_item_id", errors, WORK_RE)
    text(admission.get("capability_id"), "admission.capability_id", errors, CAPABILITY_RE)
    text(admission.get("base_sha"), "admission.base_sha", errors, SHA_RE)
    branch = text(admission.get("branch"), "admission.branch", errors)
    if branch is not None and not valid_branch(branch):
        errors.append("INVALID_BRANCH admission.branch")
    issued = timestamp(admission.get("issued_at"), "admission.issued_at", errors)
    expires = timestamp(admission.get("expires_at"), "admission.expires_at", errors)
    if issued is not None and expires is not None and issued >= expires:
        errors.append("CLAIM_TIME_ORDER")
    refs = string_list(admission.get("dependency_evidence_refs"), "admission.dependency_evidence_refs", errors)
    for index, ref in enumerate(refs):
        if not ID_RE.fullmatch(ref):
            errors.append(f"FORMAT admission.dependency_evidence_refs[{index}]")

    execution = strict_object(state.get("execution"), EXECUTION_KEYS, "execution", errors)
    phase = execution.get("phase")
    if phase not in PHASES:
        errors.append("FORMAT execution.phase")
    string_list(execution.get("completed_acceptance"), "execution.completed_acceptance", errors)
    remaining = string_list(execution.get("remaining_work"), "execution.remaining_work", errors)
    checks = execution.get("completed_checks")
    if not isinstance(checks, list):
        errors.append("TYPE execution.completed_checks: expected array")
    else:
        for index, raw_check in enumerate(checks):
            check = strict_object(raw_check, CHECK_KEYS, f"execution.completed_checks[{index}]", errors)
            text(check.get("command"), f"execution.completed_checks[{index}].command", errors)
            if check.get("outcome") not in {"passed", "failed"}:
                errors.append(f"FORMAT execution.completed_checks[{index}].outcome")

    checkpoint = strict_object(state.get("checkpoint"), CHECKPOINT_KEYS, "checkpoint", errors)
    sequence = checkpoint.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        errors.append("FORMAT checkpoint.sequence")
    kind = checkpoint.get("kind")
    if kind not in KINDS:
        errors.append("FORMAT checkpoint.kind")
    text(checkpoint.get("parent_sha"), "checkpoint.parent_sha", errors, SHA_RE)
    updated = timestamp(checkpoint.get("updated_at"), "checkpoint.updated_at", errors)
    text(checkpoint.get("next_action"), "checkpoint.next_action", errors)
    if issued is not None and updated is not None and updated < issued:
        errors.append("CHECKPOINT_BEFORE_CLAIM")
    if expires is not None and updated is not None and updated >= expires:
        errors.append("CHECKPOINT_AFTER_EXPIRY")
    if phase == "candidate" and kind != "candidate":
        errors.append("CANDIDATE_KIND_REQUIRED")
    if phase == "blocked" and kind != "blocked":
        errors.append("BLOCKED_KIND_REQUIRED")
    if phase == "candidate" and remaining:
        errors.append("CANDIDATE_REMAINING_WORK")
    return errors


def git(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args], cwd=root, text=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, check=check,
    )


def is_ancestor(root: Path, older: str, newer: str) -> bool:
    return git(root, "merge-base", "--is-ancestor", older, newer, check=False).returncode == 0


def previous_state(root: Path, parent: str, state_path: Path) -> tuple[dict[str, Any] | None, str | None]:
    shown = git(root, "show", f"{parent}:{state_path.as_posix()}", check=False)
    if shown.returncode != 0:
        return None, None
    try:
        value = json.loads(shown.stdout, object_pairs_hook=unique_object)
    except (DuplicateKey, json.JSONDecodeError):
        return None, "PREVIOUS_STATE_INVALID"
    if not isinstance(value, dict) or validate_state(value):
        return None, "PREVIOUS_STATE_INVALID"
    return value, None


def parse_now(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError("--at must include a timezone")
    return parsed


def inspect(root: Path, state_path: Path, state: dict[str, Any], at: datetime, remote_head: str) -> dict[str, Any]:
    errors = validate_state(state)
    result: dict[str, Any] = {
        "admitted": False,
        "classification": "invalid",
        "errors": errors,
        "facts": {},
        "next_action": state.get("checkpoint", {}).get("next_action"),
    }
    if errors:
        return result
    if not SHA_RE.fullmatch(remote_head):
        result["errors"].append("FORMAT remote_head")
        return result
    try:
        head = git(root, "rev-parse", "HEAD").stdout.strip()
        branch = git(root, "branch", "--show-current").stdout.strip()
        parent = git(root, "rev-parse", "HEAD^").stdout.strip()
        remote = git(root, "remote", "get-url", "origin").stdout.strip()
        dirty_lines = [line for line in git(root, "status", "--porcelain=v1", "--untracked-files=all").stdout.splitlines()]
    except subprocess.CalledProcessError as exc:
        result["errors"].append(f"GIT_ERROR {exc.stderr.strip()}")
        return result

    admission = state["admission"]
    expires = datetime.fromisoformat(admission["expires_at"].replace("Z", "+00:00"))
    facts = {
        "local_head": head,
        "remote_head": remote_head,
        "branch": branch,
        "expected_branch": admission["branch"],
        "dirty_paths": dirty_lines,
        "claim_id": admission["claim_id"],
        "claim_generation": admission["claim_generation"],
        "base_sha": admission["base_sha"],
    }
    result["facts"] = facts

    if remote != REMOTE:
        result["classification"] = "invalid"
        result["errors"].append(f"REMOTE_MISMATCH expected={REMOTE} actual={remote}")
        return result
    if at < datetime.fromisoformat(admission["issued_at"].replace("Z", "+00:00")) or at >= expires:
        result["classification"] = "expired_claim"
        result["errors"].append("CLAIM_EXPIRED_OR_NOT_STARTED")
        return result
    if not git(root, "cat-file", "-e", f"{remote_head}^{{commit}}", check=False).returncode == 0:
        result["classification"] = "stale"
        result["errors"].append("REMOTE_HEAD_UNKNOWN_LOCALLY")
        return result
    if head != remote_head:
        if is_ancestor(root, head, remote_head):
            result["classification"] = "stale"
            result["errors"].append("REMOTE_BRANCH_AHEAD")
        elif is_ancestor(root, remote_head, head):
            result["classification"] = "stale"
            result["errors"].append("LOCAL_CHECKPOINT_NOT_PUSHED")
        else:
            result["classification"] = "diverged"
            result["errors"].append("LOCAL_REMOTE_DIVERGED")
        return result
    stale: list[str] = []
    if branch != admission["branch"]:
        stale.append("BRANCH_MISMATCH")
    if parent != state["checkpoint"]["parent_sha"]:
        stale.append("STATE_NOT_UPDATED_IN_HEAD")
    if not is_ancestor(root, admission["base_sha"], head):
        stale.append("BASE_NOT_ANCESTOR")
    expected_path = Path("delivery/executions") / f"{admission['claim_id']}.json"
    if state_path != expected_path:
        stale.append("STATE_PATH_MISMATCH")
    tracked = git(root, "ls-files", "--error-unmatch", str(state_path), check=False)
    if tracked.returncode != 0:
        stale.append("STATE_NOT_TRACKED")
    prior, prior_error = previous_state(root, parent, state_path)
    if prior_error:
        stale.append(prior_error)
    elif prior is None:
        if state["checkpoint"]["sequence"] != 1:
            stale.append("INITIAL_SEQUENCE_NOT_ONE")
        if state["execution"]["phase"] != "claimed" or state["checkpoint"]["kind"] != "claim":
            stale.append("INITIAL_CHECKPOINT_NOT_CLAIMED")
    else:
        if state["repository"] != prior["repository"] or state["admission"] != prior["admission"]:
            stale.append("IMMUTABLE_BINDING_CHANGED")
        if state["checkpoint"]["sequence"] != prior["checkpoint"]["sequence"] + 1:
            stale.append("CHECKPOINT_SEQUENCE_NOT_MONOTONIC")
        allowed = {
            "claimed": {"claimed", "implementing", "checkpointed", "blocked"},
            "implementing": {"implementing", "checkpointed", "candidate", "blocked"},
            "checkpointed": {"implementing", "checkpointed", "candidate", "blocked"},
            "blocked": {"blocked"},
            "candidate": {"candidate"},
        }
        if state["execution"]["phase"] not in allowed[prior["execution"]["phase"]]:
            stale.append("INVALID_PHASE_TRANSITION")
    if stale:
        result["classification"] = "stale"
        result["errors"].extend(stale)
        return result
    if dirty_lines:
        result["classification"] = "dirty"
        result["errors"].append("WORKTREE_DIRTY")
        return result
    result["classification"] = "clean"
    result["admitted"] = True
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--state", required=True, type=Path)
    inspect_parser = subparsers.add_parser("inspect")
    inspect_parser.add_argument("--root", required=True, type=Path)
    inspect_parser.add_argument("--state", required=True, type=Path)
    inspect_parser.add_argument("--at", required=True)
    inspect_parser.add_argument("--remote-head", required=True)
    args = parser.parse_args(argv)

    errors: list[str] = []
    state = load_state(args.state, errors)
    if args.command == "validate":
        errors.extend(validate_state(state) if state else [])
        output = {"valid": not errors, "errors": errors}
        print(json.dumps(output, sort_keys=True))
        return 0 if not errors else 1

    try:
        at = parse_now(args.at)
    except ValueError as exc:
        errors.append(str(exc))
    if errors:
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": errors}, sort_keys=True))
        return 1
    root = args.root.resolve()
    try:
        relative_state = args.state.resolve().relative_to(root)
    except ValueError:
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": ["STATE_OUTSIDE_ROOT"]}, sort_keys=True))
        return 1
    result = inspect(root, relative_state, state, at, args.remote_head)
    print(json.dumps(result, sort_keys=True))
    return 0 if result["admitted"] else 1


if __name__ == "__main__":
    sys.exit(main())
