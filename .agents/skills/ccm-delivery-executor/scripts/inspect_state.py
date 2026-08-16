#!/usr/bin/env python3
"""Validate and inspect a restart-safe CCM task-branch checkpoint."""

from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
EVIDENCE_RE = re.compile(r"^evidence-[a-z0-9][a-z0-9._-]*$")
WORK_RE = re.compile(r"^CCM-[A-Z]+-[0-9]{3}$")
PUBLIC_CAPABILITY_RE = re.compile(r"^ccm\.[a-z0-9]+(?:[.-][a-z0-9]+)*\.v[1-9][0-9]*$")
REMOTE = "git@github.com:korkin25/codex-claude-mode.git"
CONTROLLER_REMOTE = "git@github.com:korkin25/ccm-multi.git"
TOP_KEYS = {"document_type", "schema_version", "repository", "admission", "execution", "checkpoint"}
REPOSITORY_KEYS = {"id", "remote", "default_branch"}
ADMISSION_KEYS = {
    "controller_commit_sha", "claim_digest", "claim_id", "claim_generation", "predecessor",
    "owner_principal", "work_item_id", "public_capability_id", "capabilities", "base_sha",
    "branch", "issued_at", "expires_at", "dependency_evidence_refs", "dependency_evidence",
}
PREDECESSOR_KEYS = {"claim_id", "generation", "checkpoint_sha"}
EVIDENCE_BINDING_KEYS = {"id", "digest"}
EXECUTION_KEYS = {"phase", "completed_acceptance", "remaining_work", "completed_checks"}
CHECKPOINT_KEYS = {"sequence", "kind", "parent_sha", "updated_at", "next_action"}
CHECK_KEYS = {"command", "outcome"}
CLAIM_KEYS = {
    "id", "work_item_id", "owner_principal", "repository_id", "base_sha", "branch",
    "capabilities", "generation", "status", "issued_at", "expires_at",
    "dependency_evidence_refs",
}
WORK_KEYS = {
    "id", "owner_repository", "capabilities", "status", "dependencies",
    "external_prerequisites", "evidence_refs",
}
EXTERNAL_KEYS = {"id", "state", "evidence_refs", "blocks"}
EVIDENCE_KEYS = {
    "id", "kind", "repository_id", "merge_sha", "content_digest", "ci",
    "check_contract", "verified_at", "verifier", "provenance",
}
CI_KEYS = {"provider", "run_id", "url", "head_sha", "status", "required_checks"}
CHECK_CONTRACT_KEYS = {"path", "format", "content_digest"}
PHASE_KIND = {
    "claimed": "claim",
    "implementing": "progress",
    "checkpointed": "planned_restart",
    "candidate": "candidate",
    "blocked": "blocked",
}


class DuplicateKey(ValueError):
    pass


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKey(key)
        result[key] = value
    return result


def canonical_digest(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def load_json_bytes(raw: bytes, where: str, errors: list[str]) -> dict[str, Any]:
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object)
    except DuplicateKey as exc:
        errors.append(f"DUPLICATE_KEY {where}: {exc}")
        return {}
    except (UnicodeError, json.JSONDecodeError) as exc:
        errors.append(f"JSON_INVALID {where}: {exc}")
        return {}
    if not isinstance(value, dict):
        errors.append(f"TYPE {where}: expected object")
        return {}
    return value


def load_state(path: Path, errors: list[str]) -> dict[str, Any]:
    try:
        return load_json_bytes(path.read_bytes(), str(path), errors)
    except OSError as exc:
        errors.append(f"STATE_UNREADABLE {exc}")
        return {}


def strict_object(value: Any, keys: set[str], where: str, errors: list[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        errors.append(f"TYPE {where}: expected object")
        return {}
    missing, extra = sorted(keys - set(value)), sorted(set(value) - keys)
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


def string_list(value: Any, where: str, errors: list[str], *, nonempty: bool = False) -> list[str]:
    if not isinstance(value, list) or (nonempty and not value):
        errors.append(f"TYPE {where}: expected {'non-empty ' if nonempty else ''}array")
        return []
    if not all(isinstance(item, str) and item for item in value):
        errors.append(f"FORMAT {where}")
        return []
    if len(value) != len(set(value)):
        errors.append(f"DUPLICATE {where}")
    return value


def valid_branch(branch: str | None) -> bool:
    return bool(
        branch and not branch.startswith((".", "/")) and not branch.endswith((".", "/", ".lock"))
        and ".." not in branch and "@{" not in branch and "//" not in branch
        and not re.search(r"[\x00-\x20~^:?*\\]", branch)
    )


def validate_state(state: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    strict_object(state, TOP_KEYS, "state", errors)
    if state.get("document_type") != "ccm-delivery-execution-state": errors.append("DOCUMENT_TYPE")
    if state.get("schema_version") != 1: errors.append("SCHEMA_VERSION")
    repository = strict_object(state.get("repository"), REPOSITORY_KEYS, "repository", errors)
    if repository.get("id") != "ccm-public": errors.append("REPOSITORY_ID")
    if repository.get("remote") != REMOTE: errors.append("REPOSITORY_REMOTE")
    if repository.get("default_branch") != "main": errors.append("DEFAULT_BRANCH")

    admission = strict_object(state.get("admission"), ADMISSION_KEYS, "admission", errors)
    text(admission.get("controller_commit_sha"), "admission.controller_commit_sha", errors, SHA_RE)
    text(admission.get("claim_digest"), "admission.claim_digest", errors, DIGEST_RE)
    text(admission.get("claim_id"), "admission.claim_id", errors, ID_RE)
    generation = admission.get("claim_generation")
    if not isinstance(generation, int) or isinstance(generation, bool) or generation < 1:
        errors.append("FORMAT admission.claim_generation")
    predecessor = admission.get("predecessor")
    if predecessor is not None:
        predecessor = strict_object(predecessor, PREDECESSOR_KEYS, "admission.predecessor", errors)
        text(predecessor.get("claim_id"), "admission.predecessor.claim_id", errors, ID_RE)
        if not isinstance(predecessor.get("generation"), int) or isinstance(predecessor.get("generation"), bool) or predecessor.get("generation", 0) < 1:
            errors.append("FORMAT admission.predecessor.generation")
        text(predecessor.get("checkpoint_sha"), "admission.predecessor.checkpoint_sha", errors, SHA_RE)
    if generation == 1 and predecessor is not None: errors.append("GENERATION_ONE_HAS_PREDECESSOR")
    if isinstance(generation, int) and generation > 1 and predecessor is None: errors.append("GENERATION_CHAIN_REQUIRED")
    if isinstance(generation, int) and generation > 1 and predecessor and predecessor.get("generation") != generation - 1:
        errors.append("GENERATION_CHAIN_NOT_IMMEDIATE")
    text(admission.get("owner_principal"), "admission.owner_principal", errors, ID_RE)
    text(admission.get("work_item_id"), "admission.work_item_id", errors, WORK_RE)
    text(admission.get("public_capability_id"), "admission.public_capability_id", errors, PUBLIC_CAPABILITY_RE)
    for index, capability in enumerate(string_list(admission.get("capabilities"), "admission.capabilities", errors, nonempty=True)):
        if not ID_RE.fullmatch(capability): errors.append(f"FORMAT admission.capabilities[{index}]")
    text(admission.get("base_sha"), "admission.base_sha", errors, SHA_RE)
    branch = text(admission.get("branch"), "admission.branch", errors)
    if branch is not None and not valid_branch(branch): errors.append("INVALID_BRANCH admission.branch")
    issued = timestamp(admission.get("issued_at"), "admission.issued_at", errors)
    expires = timestamp(admission.get("expires_at"), "admission.expires_at", errors)
    if issued is not None and expires is not None and issued >= expires: errors.append("CLAIM_TIME_ORDER")
    refs = string_list(admission.get("dependency_evidence_refs"), "admission.dependency_evidence_refs", errors)
    for index, ref in enumerate(refs):
        if not EVIDENCE_RE.fullmatch(ref): errors.append(f"FORMAT admission.dependency_evidence_refs[{index}]")
    bindings = admission.get("dependency_evidence")
    binding_ids: list[str] = []
    if not isinstance(bindings, list):
        errors.append("TYPE admission.dependency_evidence: expected array")
    else:
        for index, raw in enumerate(bindings):
            binding = strict_object(raw, EVIDENCE_BINDING_KEYS, f"admission.dependency_evidence[{index}]", errors)
            evidence_id = text(binding.get("id"), f"admission.dependency_evidence[{index}].id", errors, EVIDENCE_RE)
            text(binding.get("digest"), f"admission.dependency_evidence[{index}].digest", errors, DIGEST_RE)
            if evidence_id: binding_ids.append(evidence_id)
    if len(binding_ids) != len(set(binding_ids)): errors.append("DUPLICATE admission.dependency_evidence")
    if set(binding_ids) != set(refs): errors.append("DEPENDENCY_EVIDENCE_BINDING_SET_MISMATCH")

    execution = strict_object(state.get("execution"), EXECUTION_KEYS, "execution", errors)
    phase = execution.get("phase")
    if phase not in PHASE_KIND: errors.append("FORMAT execution.phase")
    string_list(execution.get("completed_acceptance"), "execution.completed_acceptance", errors)
    remaining = string_list(execution.get("remaining_work"), "execution.remaining_work", errors)
    checks = execution.get("completed_checks")
    if not isinstance(checks, list):
        errors.append("TYPE execution.completed_checks: expected array")
        checks = []
    for index, raw_check in enumerate(checks):
        check = strict_object(raw_check, CHECK_KEYS, f"execution.completed_checks[{index}]", errors)
        text(check.get("command"), f"execution.completed_checks[{index}].command", errors)
        if check.get("outcome") not in {"passed", "failed"}: errors.append(f"FORMAT execution.completed_checks[{index}].outcome")

    checkpoint = strict_object(state.get("checkpoint"), CHECKPOINT_KEYS, "checkpoint", errors)
    sequence = checkpoint.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1: errors.append("FORMAT checkpoint.sequence")
    kind = checkpoint.get("kind")
    if phase in PHASE_KIND and kind != PHASE_KIND[phase]: errors.append("PHASE_KIND_MISMATCH")
    text(checkpoint.get("parent_sha"), "checkpoint.parent_sha", errors, SHA_RE)
    updated = timestamp(checkpoint.get("updated_at"), "checkpoint.updated_at", errors)
    text(checkpoint.get("next_action"), "checkpoint.next_action", errors)
    if issued is not None and updated is not None and updated < issued: errors.append("CHECKPOINT_BEFORE_CLAIM")
    if expires is not None and updated is not None and updated >= expires: errors.append("CHECKPOINT_AFTER_EXPIRY")
    if phase == "candidate" and remaining: errors.append("CANDIDATE_REMAINING_WORK")
    if phase == "candidate" and any(check.get("outcome") == "failed" for check in checks if isinstance(check, dict)):
        errors.append("CANDIDATE_FAILED_CHECK")
    return errors


def git_environment() -> dict[str, str]:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_CONFIG_")}
    for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY",
                "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG_PARAMETERS", "GIT_SSH",
                "GIT_SSH_COMMAND", "GIT_ASKPASS", "SSH_ASKPASS"):
        env.pop(key, None)
    env.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_TERMINAL_PROMPT": "0", "LC_ALL": "C"})
    return env


def git(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=root, env=git_environment(), text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=check)


def git_bytes(root: Path, *args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["git", *args], cwd=root, env=git_environment(),
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)


def is_ancestor(root: Path, older: str, newer: str) -> bool:
    return git(root, "merge-base", "--is-ancestor", older, newer, check=False).returncode == 0


def remote_errors(root: Path, expected: str) -> list[str]:
    errors: list[str] = []
    commands = {
        "RAW_FETCH_URL": ("config", "--local", "--get-all", "remote.origin.url"),
        "RAW_PUSH_URL": ("config", "--local", "--get-all", "remote.origin.pushurl"),
        "EFFECTIVE_FETCH_URL": ("remote", "get-url", "--all", "origin"),
        "EFFECTIVE_PUSH_URL": ("remote", "get-url", "--push", "--all", "origin"),
    }
    values: dict[str, list[str]] = {}
    for label, command in commands.items():
        result = git(root, *command, check=False)
        values[label] = result.stdout.splitlines() if result.returncode == 0 else []
    if not values["RAW_PUSH_URL"]: values["RAW_PUSH_URL"] = values["RAW_FETCH_URL"][:]
    for label, urls in values.items():
        if urls != [expected]: errors.append(f"{label}_MISMATCH expected={expected} actual={urls}")
    rewrites = git(root, "config", "--local", "--get-regexp", r"^url\..*\.(insteadOf|pushInsteadOf)$", check=False)
    if rewrites.returncode == 0 and rewrites.stdout.strip(): errors.append("LOCAL_URL_REWRITE_FORBIDDEN")
    dangerous = git(root, "config", "--local", "--get-regexp",
                    r"^(core\.(sshCommand|fsmonitor|hooksPath)|remote\.origin\.(uploadpack|receivepack)|include\.|includeIf\.)", check=False)
    if dangerous.returncode == 0 and dangerous.stdout.strip(): errors.append("LOCAL_GIT_EXECUTION_OVERRIDE_FORBIDDEN")
    return errors


def git_json(root: Path, revision: str, path: str, errors: list[str]) -> dict[str, Any]:
    result = git_bytes(root, "show", f"{revision}:{path}")
    if result.returncode != 0:
        errors.append(f"CONTROLLER_DOCUMENT_UNAVAILABLE {path}")
        return {}
    return load_json_bytes(result.stdout, path, errors)


def unique_index(records: Any, where: str, errors: list[str]) -> dict[str, dict[str, Any]]:
    if not isinstance(records, list):
        errors.append(f"TYPE {where}: expected array")
        return {}
    result: dict[str, dict[str, Any]] = {}
    for index, record in enumerate(records):
        if not isinstance(record, dict) or not isinstance(record.get("id"), str):
            errors.append(f"FORMAT {where}[{index}]")
            continue
        if record["id"] in result: errors.append(f"DUPLICATE_ID {where}: {record['id']}")
        result[record["id"]] = record
    return result


def validate_evidence(record: dict[str, Any], at: datetime, where: str, errors: list[str]) -> None:
    strict_object(record, EVIDENCE_KEYS, where, errors)
    text(record.get("id"), f"{where}.id", errors, EVIDENCE_RE)
    if record.get("kind") not in {"merge_ci", "external_probe"}: errors.append(f"EVIDENCE_KIND {where}")
    text(record.get("repository_id"), f"{where}.repository_id", errors, ID_RE)
    text(record.get("merge_sha"), f"{where}.merge_sha", errors, SHA_RE)
    text(record.get("content_digest"), f"{where}.content_digest", errors, DIGEST_RE)
    if record.get("provenance") != "measured": errors.append(f"EVIDENCE_NOT_MEASURED {where}")
    text(record.get("verifier"), f"{where}.verifier", errors, ID_RE)
    verified = timestamp(record.get("verified_at"), f"{where}.verified_at", errors)
    if verified is not None and verified > at: errors.append(f"EVIDENCE_FROM_FUTURE {where}")
    ci = strict_object(record.get("ci"), CI_KEYS, f"{where}.ci", errors)
    if ci.get("provider") != "github-actions" or ci.get("status") != "success": errors.append(f"EVIDENCE_CI_NOT_SUCCESS {where}")
    if not isinstance(ci.get("run_id"), str) or not ci.get("run_id", "").isdigit(): errors.append(f"EVIDENCE_CI_RUN_ID {where}")
    if not isinstance(ci.get("url"), str) or not re.fullmatch(r"https://github\.com/.+/actions/runs/[0-9]+", ci.get("url", "")):
        errors.append(f"EVIDENCE_CI_URL {where}")
    if ci.get("head_sha") != record.get("merge_sha"): errors.append(f"EVIDENCE_CI_SHA_MISMATCH {where}")
    string_list(ci.get("required_checks"), f"{where}.ci.required_checks", errors, nonempty=True)
    contract = record.get("check_contract")
    if contract is not None:
        contract = strict_object(contract, CHECK_CONTRACT_KEYS, f"{where}.check_contract", errors)
        text(contract.get("path"), f"{where}.check_contract.path", errors)
        if contract.get("format") not in {"ccm-capability-manifest-v1", "aor-capability-manifest-v1"}:
            errors.append(f"EVIDENCE_CHECK_CONTRACT_FORMAT {where}")
        text(contract.get("content_digest"), f"{where}.check_contract.content_digest", errors, DIGEST_RE)


def controller_errors(root: Path, head: str, state: dict[str, Any], at: datetime) -> tuple[list[str], bool]:
    errors = remote_errors(root, CONTROLLER_REMOTE)
    unavailable = False
    admission = state["admission"]
    if not SHA_RE.fullmatch(head) or git(root, "cat-file", "-e", f"{head}^{{commit}}", check=False).returncode:
        return errors + ["CONTROLLER_HEAD_UNAVAILABLE"], True
    issuance = admission["controller_commit_sha"]
    if git(root, "cat-file", "-e", f"{issuance}^{{commit}}", check=False).returncode:
        return errors + ["CONTROLLER_ISSUANCE_UNAVAILABLE"], True
    if not is_ancestor(root, issuance, head): errors.append("CONTROLLER_ISSUANCE_NOT_ANCESTOR")
    docs: dict[str, dict[str, Any]] = {}
    for name in ("claims.json", "state.json", "evidence.json"):
        docs[name] = git_json(root, head, f"product/delivery/{name}", errors)
    issuance_claims = git_json(root, issuance, "product/delivery/claims.json", errors)
    if any(not doc for doc in (*docs.values(), issuance_claims)):
        return errors, True
    if strict_object(docs["claims.json"], {"document_type", "schema_version", "claims"}, "controller.claims", errors).get("document_type") != "claims-registry": errors.append("CONTROLLER_CLAIMS_TYPE")
    if strict_object(docs["state.json"], {"document_type", "schema_version", "repositories", "capabilities", "external_capabilities", "work_items"}, "controller.state", errors).get("document_type") != "delivery-state": errors.append("CONTROLLER_STATE_TYPE")
    if strict_object(docs["evidence.json"], {"document_type", "schema_version", "evidence"}, "controller.evidence", errors).get("document_type") != "evidence-registry": errors.append("CONTROLLER_EVIDENCE_TYPE")
    if any(doc.get("schema_version") != 1 for doc in docs.values()): errors.append("CONTROLLER_SCHEMA_VERSION")
    issuance_top = strict_object(issuance_claims, {"document_type", "schema_version", "claims"}, "controller.issuance_claims", errors)
    if issuance_top.get("document_type") != "claims-registry" or issuance_top.get("schema_version") != 1:
        errors.append("CONTROLLER_ISSUANCE_CLAIMS_TYPE")
    claims = unique_index(docs["claims.json"].get("claims"), "controller.claims", errors)
    old_claims = unique_index(issuance_claims.get("claims"), "controller.issuance_claims", errors)
    work_items = unique_index(docs["state.json"].get("work_items"), "controller.work_items", errors)
    externals = unique_index(docs["state.json"].get("external_capabilities"), "controller.external_capabilities", errors)
    evidence = unique_index(docs["evidence.json"].get("evidence"), "controller.evidence", errors)
    claim_id = admission["claim_id"]
    claim, issued_claim = claims.get(claim_id), old_claims.get(claim_id)
    if claim is None or issued_claim is None:
        errors.append("CONTROLLER_CLAIM_MISSING")
        return errors, False
    strict_object(claim, CLAIM_KEYS, f"controller.claim.{claim_id}", errors)
    if claim != issued_claim: errors.append("CONTROLLER_CLAIM_REVOKED_OR_CHANGED")
    if canonical_digest(claim) != admission["claim_digest"]: errors.append("CLAIM_DIGEST_MISMATCH")
    expected = {
        "id": admission["claim_id"], "work_item_id": admission["work_item_id"],
        "owner_principal": admission["owner_principal"], "repository_id": "ccm-public",
        "base_sha": admission["base_sha"], "branch": admission["branch"],
        "capabilities": admission["capabilities"], "generation": admission["claim_generation"],
        "status": "active", "issued_at": admission["issued_at"], "expires_at": admission["expires_at"],
        "dependency_evidence_refs": admission["dependency_evidence_refs"],
    }
    if claim != expected: errors.append("CLAIM_BINDING_MISMATCH")
    claim_issued = timestamp(claim.get("issued_at"), "controller.claim.issued_at", errors)
    claim_expires = timestamp(claim.get("expires_at"), "controller.claim.expires_at", errors)
    if claim_issued is not None and claim_expires is not None and not claim_issued <= at < claim_expires:
        errors.append("CONTROLLER_CLAIM_EXPIRED_OR_NOT_STARTED")
    active = [item for item in claims.values() if item.get("status") == "active"]
    if active != [claim]: errors.append("CONTROLLER_ACTIVE_CLAIM_CONFLICT")
    later = [item for item in claims.values() if item.get("work_item_id") == claim.get("work_item_id")
             and item.get("repository_id") == "ccm-public" and isinstance(item.get("generation"), int)
             and item["generation"] > claim["generation"]]
    if later: errors.append("CONTROLLER_CLAIM_SUPERSEDED")
    work = work_items.get(admission["work_item_id"])
    if work is None:
        errors.append("CONTROLLER_WORK_ITEM_MISSING")
    else:
        strict_object(work, WORK_KEYS, f"controller.work.{admission['work_item_id']}", errors)
        if work.get("owner_repository") != "ccm-public" or work.get("status") != "ready": errors.append("CONTROLLER_WORK_NOT_READY")
        if work.get("capabilities") != admission["capabilities"]: errors.append("CONTROLLER_WORK_CAPABILITIES_MISMATCH")
        required: set[str] = set()
        for dependency in work.get("dependencies", []):
            dependency_work = work_items.get(dependency)
            if dependency_work is None or dependency_work.get("status") != "done": errors.append(f"CONTROLLER_DEPENDENCY_NOT_DONE {dependency}")
            else: required.update(dependency_work.get("evidence_refs", []))
        for external_id in work.get("external_prerequisites", []):
            external = externals.get(external_id)
            if external is None:
                errors.append(f"CONTROLLER_EXTERNAL_MISSING {external_id}")
            else:
                strict_object(external, EXTERNAL_KEYS, f"controller.external.{external_id}", errors)
                if external.get("state") != "available": errors.append(f"CONTROLLER_EXTERNAL_UNAVAILABLE {external_id}")
                required.update(external.get("evidence_refs", []))
        if not required.issubset(set(admission["dependency_evidence_refs"])): errors.append("CLAIM_REQUIRED_EVIDENCE_MISSING")
    bindings = {item["id"]: item["digest"] for item in admission["dependency_evidence"]}
    for evidence_id in admission["dependency_evidence_refs"]:
        record = evidence.get(evidence_id)
        if record is None:
            errors.append(f"CONTROLLER_EVIDENCE_MISSING {evidence_id}")
            continue
        validate_evidence(record, at, f"controller.evidence.{evidence_id}", errors)
        if canonical_digest(record) != bindings.get(evidence_id): errors.append(f"EVIDENCE_DIGEST_MISMATCH {evidence_id}")
    predecessor = admission["predecessor"]
    if predecessor:
        previous_claim = claims.get(predecessor["claim_id"])
        if previous_claim is None or previous_claim.get("generation") != predecessor["generation"]:
            errors.append("PREDECESSOR_CLAIM_MISSING")
        elif previous_claim.get("status") == "active" or previous_claim.get("work_item_id") != claim.get("work_item_id") or previous_claim.get("repository_id") != "ccm-public":
            errors.append("PREDECESSOR_CLAIM_NOT_CLOSED_OR_MISMATCHED")
    return errors, unavailable


def previous_state(root: Path, revision: str, path: Path) -> dict[str, Any] | None:
    result = git_bytes(root, "show", f"{revision}:{path.as_posix()}")
    if result.returncode != 0: return None
    errors: list[str] = []
    value = load_json_bytes(result.stdout, str(path), errors)
    return value if value and not validate_state(value) else None


def exact_state_errors(root: Path, head: str, path: Path) -> list[str]:
    errors: list[str] = []
    tree = git(root, "ls-tree", head, "--", path.as_posix(), check=False)
    lines = tree.stdout.splitlines()
    if len(lines) != 1 or not lines[0].startswith("100644 blob "):
        errors.append("STATE_HEAD_MODE_OR_ENTRY_MISMATCH")
    blob = git_bytes(root, "show", f"{head}:{path.as_posix()}")
    try: worktree = (root / path).read_bytes()
    except OSError: worktree = b""
    if blob.returncode != 0 or blob.stdout != worktree: errors.append("STATE_BYTES_DIFFER_FROM_HEAD")
    flags = git(root, "ls-files", "-v", "--", path.as_posix(), check=False).stdout.splitlines()
    if len(flags) != 1 or not flags[0].startswith("H "): errors.append("STATE_INDEX_FLAG_FORBIDDEN")
    return errors


def append_only_errors(prior: dict[str, Any], state: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if state["execution"]["completed_acceptance"][:len(prior["execution"]["completed_acceptance"])] != prior["execution"]["completed_acceptance"]:
        errors.append("COMPLETED_ACCEPTANCE_NOT_APPEND_ONLY")
    if state["execution"]["completed_checks"][:len(prior["execution"]["completed_checks"])] != prior["execution"]["completed_checks"]:
        errors.append("COMPLETED_CHECKS_NOT_APPEND_ONLY")
    old_time = datetime.fromisoformat(prior["checkpoint"]["updated_at"].replace("Z", "+00:00"))
    new_time = datetime.fromisoformat(state["checkpoint"]["updated_at"].replace("Z", "+00:00"))
    if new_time <= old_time: errors.append("CHECKPOINT_TIME_NOT_MONOTONIC")
    return errors


def inspect(root: Path, state_path: Path, state: dict[str, Any], at: datetime, remote_head: str,
            controller_root: Path | None = None, controller_head: str | None = None) -> dict[str, Any]:
    errors = validate_state(state)
    result: dict[str, Any] = {"admitted": False, "classification": "invalid", "errors": errors,
                              "facts": {}, "next_action": state.get("checkpoint", {}).get("next_action")}
    if errors: return result
    if not SHA_RE.fullmatch(remote_head):
        result["errors"].append("FORMAT remote_head")
        return result
    try:
        head = git(root, "rev-parse", "HEAD").stdout.strip()
        branch = git(root, "branch", "--show-current").stdout.strip()
        parent = git(root, "rev-parse", "HEAD^").stdout.strip()
        dirty = git(root, "status", "--porcelain=v1", "--untracked-files=all").stdout.splitlines()
    except subprocess.CalledProcessError as exc:
        result["errors"].append(f"GIT_ERROR {exc.stderr.strip()}")
        return result
    admission = state["admission"]
    result["facts"] = {"local_head": head, "remote_head": remote_head, "branch": branch,
        "expected_branch": admission["branch"], "dirty_paths": dirty, "claim_id": admission["claim_id"],
        "claim_generation": admission["claim_generation"], "base_sha": admission["base_sha"]}
    local_invalid = remote_errors(root, REMOTE) + exact_state_errors(root, head, state_path)
    if local_invalid:
        result["errors"].extend(local_invalid)
        return result
    issued = datetime.fromisoformat(admission["issued_at"].replace("Z", "+00:00"))
    expires = datetime.fromisoformat(admission["expires_at"].replace("Z", "+00:00"))
    updated = datetime.fromisoformat(state["checkpoint"]["updated_at"].replace("Z", "+00:00"))
    if not issued <= at < expires:
        result["classification"] = "expired_claim"
        result["errors"].append("CLAIM_EXPIRED_OR_NOT_STARTED")
        return result
    if updated > at:
        result["errors"].append("CHECKPOINT_FROM_FUTURE")
        return result
    if git(root, "cat-file", "-e", f"{remote_head}^{{commit}}", check=False).returncode:
        result["classification"] = "stale"; result["errors"].append("REMOTE_HEAD_UNKNOWN_LOCALLY"); return result
    if head != remote_head:
        if is_ancestor(root, head, remote_head): result["classification"], result["errors"] = "stale", ["REMOTE_BRANCH_AHEAD"]
        elif is_ancestor(root, remote_head, head): result["classification"], result["errors"] = "stale", ["LOCAL_CHECKPOINT_NOT_PUSHED"]
        else: result["classification"], result["errors"] = "diverged", ["LOCAL_REMOTE_DIVERGED"]
        return result
    stale: list[str] = []
    if branch != admission["branch"]: stale.append("BRANCH_MISMATCH")
    if parent != state["checkpoint"]["parent_sha"]: stale.append("STATE_NOT_UPDATED_IN_HEAD")
    if not is_ancestor(root, admission["base_sha"], head): stale.append("BASE_NOT_ANCESTOR")
    expected_path = Path("delivery/executions") / f"{admission['claim_id']}.json"
    if state_path != expected_path: stale.append("STATE_PATH_MISMATCH")
    prior = previous_state(root, parent, state_path)
    predecessor = admission["predecessor"]
    if prior is None:
        if state["checkpoint"]["sequence"] != 1: stale.append("INITIAL_SEQUENCE_NOT_ONE")
        if state["execution"]["phase"] != "claimed": stale.append("INITIAL_CHECKPOINT_NOT_CLAIMED")
        if admission["claim_generation"] == 1:
            if parent != admission["base_sha"]: stale.append("GENERATION_ONE_PARENT_NOT_BASE")
        else:
            if not predecessor or parent != predecessor["checkpoint_sha"]: stale.append("GENERATION_CHAIN_PARENT_MISMATCH")
            else:
                predecessor_state = previous_state(root, predecessor["checkpoint_sha"], Path("delivery/executions") / f"{predecessor['claim_id']}.json")
                if predecessor_state is None:
                    stale.append("GENERATION_PREDECESSOR_STATE_MISSING")
                else:
                    if predecessor_state["admission"]["claim_generation"] != predecessor["generation"]:
                        stale.append("GENERATION_PREDECESSOR_BINDING_MISMATCH")
                    stale.extend(append_only_errors(predecessor_state, state))
    else:
        if state["repository"] != prior["repository"] or state["admission"] != prior["admission"]: stale.append("IMMUTABLE_BINDING_CHANGED")
        if state["checkpoint"]["sequence"] != prior["checkpoint"]["sequence"] + 1: stale.append("CHECKPOINT_SEQUENCE_NOT_MONOTONIC")
        stale.extend(append_only_errors(prior, state))
        allowed = {"claimed": {"implementing", "checkpointed", "blocked"},
                   "implementing": {"implementing", "checkpointed", "candidate", "blocked"},
                   "checkpointed": {"implementing", "checkpointed", "candidate", "blocked"},
                   "blocked": {"blocked"}, "candidate": {"candidate"}}
        if state["execution"]["phase"] not in allowed[prior["execution"]["phase"]]: stale.append("INVALID_PHASE_TRANSITION")
    if stale:
        result["classification"] = "stale"; result["errors"].extend(stale); return result
    if dirty:
        result["classification"] = "dirty"; result["errors"].append("WORKTREE_DIRTY"); return result
    manifest_errors: list[str] = []
    manifest = git_json(root, head, "delivery/capabilities.json", manifest_errors)
    matches = [item for item in manifest.get("capabilities", []) if isinstance(item, dict)
               and item.get("id") == admission["public_capability_id"]
               and item.get("work_item") == admission["work_item_id"]]
    if manifest_errors or len(matches) != 1 or matches[0].get("status") != "ready":
        result["errors"].extend(manifest_errors or ["PUBLIC_CAPABILITY_NOT_READY_OR_MISMATCHED"])
        return result
    if controller_root is None or controller_head is None:
        result["classification"] = "local_only"; result["errors"].append("CONTROLLER_SNAPSHOT_REQUIRED"); return result
    if not controller_root.is_dir():
        result["classification"] = "local_only"; result["errors"].append("CONTROLLER_ROOT_UNAVAILABLE"); return result
    control_errors, unavailable = controller_errors(controller_root, controller_head, state, at)
    if control_errors:
        result["classification"] = "local_only" if unavailable else "invalid"
        result["errors"].extend(control_errors)
        return result
    result["classification"], result["admitted"] = "clean", True
    return result


def parse_now(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None or parsed.utcoffset() is None: raise ValueError("--at must include a timezone")
    return parsed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    validate_parser = commands.add_parser("validate")
    validate_parser.add_argument("--state", required=True, type=Path)
    inspect_parser = commands.add_parser("inspect")
    inspect_parser.add_argument("--root", required=True, type=Path)
    inspect_parser.add_argument("--state", required=True, type=Path)
    inspect_parser.add_argument("--at", required=True)
    inspect_parser.add_argument("--remote-head", required=True)
    inspect_parser.add_argument("--controller-root", type=Path)
    inspect_parser.add_argument("--controller-head")
    args = parser.parse_args(argv)
    errors: list[str] = []
    state_path = args.state if args.command == "validate" or args.state.is_absolute() else args.root / args.state
    state = load_state(state_path, errors)
    if args.command == "validate":
        errors.extend(validate_state(state) if state else [])
        print(json.dumps({"valid": not errors, "errors": errors}, sort_keys=True))
        return 0 if not errors else 1
    try: at = parse_now(args.at)
    except ValueError as exc: errors.append(str(exc)); at = datetime.min
    if errors:
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": errors}, sort_keys=True)); return 1
    root = args.root.resolve()
    try: relative_state = state_path.resolve().relative_to(root)
    except ValueError:
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": ["STATE_OUTSIDE_ROOT"]}, sort_keys=True)); return 1
    result = inspect(root, relative_state, state, at, args.remote_head,
                     args.controller_root.resolve() if args.controller_root else None, args.controller_head)
    print(json.dumps(result, sort_keys=True))
    return 0 if result["admitted"] else 1


if __name__ == "__main__": sys.exit(main())
