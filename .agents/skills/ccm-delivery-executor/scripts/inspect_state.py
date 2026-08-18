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
import selectors
import stat
import subprocess
import sys
import time
from typing import Any, NamedTuple


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
EVIDENCE_RE = re.compile(r"^evidence-[a-z0-9][a-z0-9._-]*$")
WORK_RE = re.compile(r"^CCM-[A-Z]+-[0-9]{3}$")
PUBLIC_CAPABILITY_RE = re.compile(r"^ccm\.[a-z0-9]+(?:[.-][a-z0-9]+)*\.v[1-9][0-9]*$")
REMOTE = "git@github.com:korkin25/codex-claude-mode.git"
CONTROLLER_REMOTE = "git@github.com:korkin25/ccm-multi.git"
GIT_CANDIDATES = (Path("/usr/bin/git"), Path("/bin/git"))
GIT_TIMEOUT_SECONDS = 5.0
GIT_OUTPUT_LIMIT = 4 * 1024 * 1024
TOP_KEYS = {"document_type", "schema_version", "repository", "admission", "execution", "checkpoint"}
REPOSITORY_KEYS = {"id", "remote", "default_branch"}
ADMISSION_KEYS = {
    "controller_commit_sha", "claim_digest", "claim_id", "claim_generation", "predecessor",
    "owner_principal", "work_item_id", "public_capability_id", "capabilities", "base_sha",
    "branch", "issued_at", "expires_at", "dependency_evidence_refs", "dependency_evidence",
    "acceptance_digest", "contract_digest", "inspector_digest", "required_checks",
}
PREDECESSOR_KEYS = {"claim_id", "generation", "checkpoint_sha"}
EVIDENCE_BINDING_KEYS = {"id", "digest"}
EXECUTION_KEYS = {"phase", "completed_acceptance", "remaining_work", "completed_checks"}
CHECKPOINT_KEYS = {"sequence", "kind", "parent_sha", "updated_at", "next_action"}
CHECK_KEYS = {"name", "command", "outcome"}
CLAIM_KEYS = {
    "id", "work_item_id", "owner_principal", "repository_id", "base_sha", "branch",
    "capabilities", "generation", "status", "issued_at", "expires_at",
    "dependency_evidence_refs",
}
# Bounded parallel lanes narrow a claim below its repository. The fields stay optional:
# the registry is append-only and keeps records issued before scoping existed.
CLAIM_OPTIONAL_KEYS = {"write_paths", "content_scopes"}
CLAIM_STATUSES = {"active", "released", "revoked", "expired"}
TERMINAL_CLAIM_STATUSES = {"released", "revoked", "expired"}
WORK_KEYS = {
    "id", "owner_repository", "capabilities", "status", "dependencies",
    "external_prerequisites", "evidence_refs",
}
CONTROLLER_REPOSITORY_KEYS = {"id", "role", "url", "baseline_sha"}
CAPABILITY_KEYS = {"id", "owner_repository", "write_exclusive", "scope"}
EXTERNAL_KEYS = {"id", "state", "evidence_refs", "blocks"}
EVIDENCE_KEYS = {
    "id", "kind", "repository_id", "merge_sha", "content_digest", "ci",
    "check_contract", "verified_at", "verifier", "provenance",
}
# The controller stamps an explicit supersession chain onto AOR baseline evidence. The
# key stays optional: the registry is append-only and keeps baseline records written
# before the lineage existed, so demanding it would reject their issuance snapshots.
EVIDENCE_OPTIONAL_KEYS = {"lineage"}
EVIDENCE_LINEAGE_KEYS = {"type", "generation", "supersedes"}
EVIDENCE_LINEAGE_TYPES = {"aor_baseline"}
AOR_BASELINE_PREFIX = "evidence-aor-baseline-"
CI_KEYS = {"provider", "run_id", "url", "head_sha", "status", "required_checks"}
CHECK_CONTRACT_KEYS = {"path", "format", "content_digest"}
PHASE_KIND = {
    "claimed": "claim",
    "implementing": "progress",
    "checkpointed": "planned_restart",
    "candidate": "candidate",
    "blocked": "blocked",
}
REPOSITORY_WEB = {
    "ccm-public": "https://github.com/korkin25/codex-claude-mode",
    "ccm-multi": "https://github.com/korkin25/ccm-multi",
    "aor": "https://github.com/korkin25/agent-orchestrator",
}
NORMATIVE_CHECKS = {
    "ccm-public": {"Public capability manifest", "Rust 1.95 · linux-x86_64", "Rust 1.95 · macos-arm64"},
    "ccm-multi": {"Delivery governance", "Public capability manifest", "Rust 1.95 · linux-x86_64", "Rust 1.95 · macos-arm64"},
    "aor": {"capability-governance (ubuntu-latest)", "capability-governance (macos-latest)"},
}
# A GitHub check name comes from the workflow in the merged tree, so a merge cannot have
# run a job that was added to its repository later. NORMATIVE_CHECKS is the contract in
# force today; these enumerated pre-contract merges carry the complete set their own
# `.github/workflows/ci.yml` declared, measured at that exact immutable commit. The list
# is closed: the controller registry is append-only and every record settled after a
# contract grew carries NORMATIVE_CHECKS, so a new merge can never present a reduced set.
BASELINE_RUST_CHECKS = frozenset({"Rust 1.95 · linux-x86_64", "Rust 1.95 · macos-arm64"})
HISTORICAL_CHECKS = {
    # codex-claude-mode before 8e33ddf added the "Public capability manifest" job.
    ("ccm-public", "5db306d8a8d486b6047d8e2e944181c320f6c504"): BASELINE_RUST_CHECKS,
    ("ccm-public", "2c5382035ecf84724fe796332ed1c252f1fb0bce"): BASELINE_RUST_CHECKS,
    # ccm-multi before it gained the "Delivery governance" and manifest jobs.
    ("ccm-multi", "c8c1589db37ae7c5f0625994d23886c7dc1afb36"): BASELINE_RUST_CHECKS,
}
# The same merges seen from the other side of CHECK_CONTRACT: 8e33ddf also created
# `delivery/capabilities.json`, so a merge older than that file has no manifest to bind
# and its null check_contract is the measured truth rather than a withheld binding.
# Every entry is also an enumerated pre-contract merge above.
HISTORICAL_UNCONTRACTED = frozenset({
    ("ccm-public", "5db306d8a8d486b6047d8e2e944181c320f6c504"),
    ("ccm-public", "2c5382035ecf84724fe796332ed1c252f1fb0bce"),
})
CHECK_CONTRACT = {
    "ccm-public": ("delivery/capabilities.json", "ccm-capability-manifest-v1"),
    "aor": (".delivery/capabilities.json", "aor-capability-manifest-v1"),
}
REPOSITORY_ROLES = {"public_client", "authoritative_runtime", "product_integration"}
WORK_STATUSES = {"planned", "ready", "blocked", "done"}
EXTERNAL_STATES = {"available", "unavailable", "unknown"}


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


def bytes_digest(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


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


def strict_object(value: Any, keys: set[str], where: str, errors: list[str],
                  *, optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict):
        errors.append(f"TYPE {where}: expected object")
        return {}
    missing = sorted(keys - set(value))
    extra = sorted(set(value) - keys - (optional or set()))
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


def schema_version_one(document: dict[str, Any]) -> bool:
    value = document.get("schema_version")
    return type(value) is int and value == 1


def validate_state(state: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    strict_object(state, TOP_KEYS, "state", errors)
    if state.get("document_type") != "ccm-delivery-execution-state": errors.append("DOCUMENT_TYPE")
    if not schema_version_one(state): errors.append("SCHEMA_VERSION")
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
    text(admission.get("acceptance_digest"), "admission.acceptance_digest", errors, DIGEST_RE)
    text(admission.get("contract_digest"), "admission.contract_digest", errors, DIGEST_RE)
    text(admission.get("inspector_digest"), "admission.inspector_digest", errors, DIGEST_RE)
    string_list(admission.get("required_checks"), "admission.required_checks", errors, nonempty=True)

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
        text(check.get("name"), f"execution.completed_checks[{index}].name", errors)
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


class GitProbeError(RuntimeError):
    pass


class GitResult(NamedTuple):
    returncode: int
    stdout: bytes
    stderr: bytes


class LocalConfigEntry(NamedTuple):
    key: str
    value: str


def _strip_config_comment(value: str) -> str:
    """Strip a Git-config comment without interpreting or executing config."""
    quoted = False
    escaped = False
    for index, character in enumerate(value):
        if escaped:
            escaped = False
        elif character == "\\" and quoted:
            escaped = True
        elif character == '"':
            quoted = not quoted
        elif character in "#;" and not quoted:
            return value[:index].rstrip()
    if quoted or escaped:
        raise ValueError("unterminated quoted value")
    return value.rstrip()


def _config_value(raw: str) -> str:
    value = _strip_config_comment(raw).strip()
    if not value.startswith('"'):
        if '"' in value or "\\" in value:
            raise ValueError("unsupported unquoted escape")
        return value
    if len(value) < 2 or not value.endswith('"'):
        raise ValueError("unterminated quoted value")
    decoded: list[str] = []
    escaped = False
    escapes = {'"': '"', "\\": "\\", "n": "\n", "t": "\t", "b": "\b"}
    for character in value[1:-1]:
        if escaped:
            if character not in escapes:
                raise ValueError("unsupported quoted escape")
            decoded.append(escapes[character]); escaped = False
        elif character == "\\":
            escaped = True
        elif character == '"':
            raise ValueError("unescaped quote")
        else:
            decoded.append(character)
    if escaped:
        raise ValueError("unterminated escape")
    return "".join(decoded)


def parse_local_config(raw: bytes) -> list[LocalConfigEntry]:
    """Parse the inert subset used by repository-local config, failing closed."""
    if len(raw) > GIT_OUTPUT_LIMIT or b"\x00" in raw:
        raise ValueError("config size or NUL")
    text_value = raw.decode("utf-8")
    if any(line.rstrip("\r").endswith("\\") for line in text_value.splitlines()):
        raise ValueError("continued config line")
    section: str | None = None
    entries: list[LocalConfigEntry] = []
    section_re = re.compile(r'^\s*\[([A-Za-z0-9][A-Za-z0-9.-]*)(?:\s+"([^"\\\x00-\x1f]*)")?\]\s*(?:[#;].*)?$')
    variable_re = re.compile(r"^\s*([A-Za-z][A-Za-z0-9-]*)(?:\s*=\s*(.*))?$")
    for source_line in text_value.splitlines():
        line = source_line.rstrip("\r")
        if not line.strip() or line.lstrip().startswith(("#", ";")):
            continue
        match = section_re.fullmatch(line)
        if match:
            section = match.group(1).lower()
            if match.group(2) is not None:
                section += "." + match.group(2)
            continue
        if section is None:
            raise ValueError("variable before section")
        match = variable_re.fullmatch(line)
        if not match:
            raise ValueError("unsupported config syntax")
        value = "true" if match.group(2) is None else _config_value(match.group(2))
        entries.append(LocalConfigEntry(f"{section}.{match.group(1).lower()}", value))
    return entries


def local_config(root: Path) -> list[LocalConfigEntry]:
    path = root / ".git/config"
    try:
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
            raise ValueError("unsafe config path")
        return parse_local_config(path.read_bytes())
    except (OSError, UnicodeError, ValueError) as exc:
        raise GitProbeError("LOCAL_CONFIG_UNREADABLE") from exc


def trusted_git() -> Path:
    effective_uid = os.geteuid()
    for candidate in GIT_CANDIDATES:
        try:
            resolved = candidate.resolve(strict=True)
            nodes = [resolved, *resolved.parents]
            metadata = [node.lstat() for node in nodes]
        except OSError:
            continue
        owner = metadata[0].st_uid
        if owner == effective_uid or not stat.S_ISREG(metadata[0].st_mode) or not metadata[0].st_mode & 0o111:
            continue
        if all(item.st_uid == owner and not item.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
                   and (index == 0 or stat.S_ISDIR(item.st_mode)) for index, item in enumerate(metadata)):
            return resolved
    raise GitProbeError("TRUSTED_GIT_UNAVAILABLE")


def git_environment() -> dict[str, str]:
    return {"GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1", "GIT_TERMINAL_PROMPT": "0",
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_ATTR_NOSYSTEM": "1",
            "GIT_OPTIONAL_LOCKS": "0", "HOME": os.devnull, "XDG_CONFIG_HOME": os.devnull, "LC_ALL": "C"}


def git_command(args: tuple[str, ...]) -> list[str]:
    return [str(trusted_git()), "--no-pager", "--no-optional-locks",
            "-c", "core.fsmonitor=false",
            "-c", "diff.external=", "-c", "diff.trustExitCode=false", "-c", "core.attributesFile=/dev/null",
            "-c", "protocol.ext.allow=never", "-c", "protocol.file.allow=never", *args]


def bounded_git(root: Path, *args: str) -> GitResult:
    try:
        process = subprocess.Popen(git_command(args), cwd=root, env=git_environment(),
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except OSError as exc:
        raise GitProbeError("GIT_UNAVAILABLE") from exc
    assert process.stdout is not None and process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    chunks: dict[str, list[bytes]] = {"stdout": [], "stderr": []}
    sizes = {"stdout": 0, "stderr": 0}
    deadline = time.monotonic() + GIT_TIMEOUT_SECONDS
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                process.kill(); process.wait(); raise GitProbeError("GIT_TIMEOUT")
            for key, _ in selector.select(remaining):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj); continue
                label = key.data; sizes[label] += len(chunk)
                if sizes[label] > GIT_OUTPUT_LIMIT:
                    process.kill(); process.wait(); raise GitProbeError(f"GIT_{label.upper()}_LIMIT")
                chunks[label].append(chunk)
        returncode = process.wait(timeout=max(0.01, deadline - time.monotonic()))
    except subprocess.TimeoutExpired as exc:
        process.kill(); process.wait(); raise GitProbeError("GIT_TIMEOUT") from exc
    finally:
        selector.close()
        process.stdout.close()
        process.stderr.close()
    return GitResult(returncode, b"".join(chunks["stdout"]), b"".join(chunks["stderr"]))


def git(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    try:
        result = bounded_git(root, *args)
    except GitProbeError as exc:
        result = GitResult(125, b"", str(exc).encode())
    stdout, stderr = result.stdout.decode("utf-8", "strict"), result.stderr.decode("utf-8", "replace")
    completed = subprocess.CompletedProcess(["trusted-git", *args], result.returncode, stdout, stderr)
    if check and result.returncode: raise subprocess.CalledProcessError(result.returncode, completed.args, stdout, stderr)
    return completed


def git_bytes(root: Path, *args: str) -> GitResult:
    try:
        return bounded_git(root, *args)
    except GitProbeError as exc:
        return GitResult(125, b"", str(exc).encode())


def is_ancestor(root: Path, older: str, newer: str) -> bool:
    return git(root, "merge-base", "--is-ancestor", older, newer, check=False).returncode == 0


def repository_guard(root: Path, expected_name: str | None = None) -> list[str]:
    errors: list[str] = []
    try:
        absolute = Path(os.path.abspath(root))
        resolved = root.resolve(strict=True)
        if absolute != resolved: errors.append("REPOSITORY_ROOT_NOT_CANONICAL")
        if expected_name is not None and resolved.name != expected_name: errors.append("CONTROLLER_ROOT_NAME_MISMATCH")
        git_dir = resolved / ".git"
        if not stat.S_ISDIR(git_dir.lstat().st_mode) or git_dir.is_symlink(): errors.append("GIT_DIR_NOT_PRIVATE_DIRECTORY")
        for name in ("config", "index"):
            node = git_dir / name
            if not stat.S_ISREG(node.lstat().st_mode) or node.is_symlink(): errors.append(f"GIT_{name.upper()}_NOT_REGULAR")
        if not stat.S_ISDIR((git_dir / "objects").lstat().st_mode) or (git_dir / "objects").is_symlink():
            errors.append("GIT_OBJECTS_NOT_PRIVATE_DIRECTORY")
        forbidden = [git_dir / "shallow", git_dir / "commondir", git_dir / "gitdir", git_dir / "index.lock",
                     git_dir / "info/grafts", git_dir / "objects/info/alternates",
                     git_dir / "objects/info/http-alternates", git_dir / "info/sparse-checkout"]
        if any(path.exists() or path.is_symlink() for path in forbidden): errors.append("GIT_ALTERNATE_SHALLOW_OR_SPARSE_STATE_FORBIDDEN")
        replace_dir = git_dir / "refs/replace"
        if replace_dir.exists() and any(replace_dir.iterdir()): errors.append("GIT_REPLACE_REFS_FORBIDDEN")
        if list(git_dir.glob("sharedindex.*")): errors.append("GIT_SPLIT_INDEX_FORBIDDEN")
    except OSError:
        return errors + ["REPOSITORY_LAYOUT_UNAVAILABLE"]
    if errors: return errors
    try:
        config = local_config(resolved)
    except GitProbeError:
        return ["LOCAL_CONFIG_UNREADABLE"]
    forbidden_key = re.compile(
        r"^(include\.|includeif\.|url\.|core\.(sshcommand|fsmonitor|hookspath|worktree|alternaterefscommand|sparsecheckout|splitindex)|"
        r"diff\.|filter\.|remote\..*\.(uploadpack|receivepack|promisor|partialclonefilter)|extensions\.(partialclone|worktreeconfig)|index\.sparse)", re.I)
    if any(forbidden_key.match(entry.key) for entry in config): errors.append("LOCAL_GIT_ACTIVE_CONFIG_FORBIDDEN")
    return errors


def remote_errors(root: Path, expected: str) -> list[str]:
    errors: list[str] = []
    try:
        entries = local_config(root.resolve(strict=True))
    except (OSError, GitProbeError):
        return ["LOCAL_CONFIG_UNREADABLE"]
    values = {
        "RAW_FETCH_URL": [entry.value for entry in entries if entry.key == "remote.origin.url"],
        "RAW_PUSH_URL": [entry.value for entry in entries if entry.key == "remote.origin.pushurl"],
    }
    if not values["RAW_PUSH_URL"]: values["RAW_PUSH_URL"] = values["RAW_FETCH_URL"][:]
    values["EFFECTIVE_FETCH_URL"] = values["RAW_FETCH_URL"][:]
    values["EFFECTIVE_PUSH_URL"] = values["RAW_PUSH_URL"][:]
    for label, urls in values.items():
        if urls != [expected]: errors.append(f"{label}_MISMATCH expected={expected} actual={urls}")
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


def normative_check_sets(repository_id: Any, merge_sha: Any) -> set[frozenset[str]]:
    """Every required-check set one exact merge of one repository may normatively carry.

    The current contract always applies. An enumerated pre-contract merge additionally
    admits the complete set its own workflow declared, keyed by the immutable merge SHA
    rather than by a self-reported timestamp, so no other record can borrow it.
    """
    current = NORMATIVE_CHECKS.get(repository_id)
    allowed = {frozenset(current)} if current is not None else set()
    historical = HISTORICAL_CHECKS.get((repository_id, merge_sha))
    if historical is not None:
        allowed.add(historical)
    return allowed


def validate_evidence_lineage(record: dict[str, Any], where: str, errors: list[str]) -> None:
    """Mirror the controller's per-record AOR baseline lineage rules.

    The controller admits `lineage` only on AOR baseline evidence and requires the exact
    triple there, so an unknown key inside it stays an error just like anywhere else.
    """
    lineage_where = f"{where}.lineage"
    evidence_id = record.get("id")
    if (not isinstance(evidence_id, str) or not evidence_id.startswith(AOR_BASELINE_PREFIX)
            or record.get("repository_id") != "aor"):
        errors.append(f"EVIDENCE_LINEAGE_SCOPE {where}")
    lineage = strict_object(record.get("lineage"), EVIDENCE_LINEAGE_KEYS, lineage_where, errors)
    if not lineage:
        return
    if lineage.get("type") not in EVIDENCE_LINEAGE_TYPES:
        errors.append(f"EVIDENCE_LINEAGE_TYPE {lineage_where}")
    generation = lineage.get("generation")
    if not isinstance(generation, int) or isinstance(generation, bool) or generation < 1:
        errors.append(f"FORMAT {lineage_where}.generation")
        generation = None
    supersedes = lineage.get("supersedes")
    if supersedes is not None and (not isinstance(supersedes, str)
                                   or not EVIDENCE_RE.fullmatch(supersedes)):
        errors.append(f"FORMAT {lineage_where}.supersedes")
    elif generation is not None and (generation == 1) != (supersedes is None):
        # Generation one opens the chain and supersedes nothing; every later generation
        # names the record it replaces.
        errors.append(f"EVIDENCE_LINEAGE_ORIGIN {lineage_where}")


def validate_evidence_lineage_chain(evidence: dict[str, dict[str, Any]], label: str,
                                    errors: list[str]) -> None:
    """Mirror the controller's append-only supersession chain across one snapshot.

    Only records carrying a well-formed lineage take part: validate_evidence already
    reports malformed ones, and a snapshot older than the lineage carries none at all.
    A cycle cannot survive these rules, because contiguous generations plus "each record
    names its immediate predecessor" admit exactly one line per lineage type.
    """
    chains: dict[str, dict[int, list[str]]] = {}
    supersedes_by_id: dict[str, str | None] = {}
    for evidence_id, record in sorted(evidence.items()):
        lineage = record.get("lineage")
        if not isinstance(lineage, dict):
            continue
        kind, generation, supersedes = (lineage.get("type"), lineage.get("generation"),
                                        lineage.get("supersedes"))
        if (kind not in EVIDENCE_LINEAGE_TYPES or not isinstance(generation, int)
                or isinstance(generation, bool) or generation < 1
                or (supersedes is not None and not isinstance(supersedes, str))):
            continue
        if supersedes is not None and supersedes not in evidence:
            errors.append(f"CONTROLLER_EVIDENCE_LINEAGE_MISSING {label}.evidence.{evidence_id} {supersedes}")
        chains.setdefault(kind, {}).setdefault(generation, []).append(evidence_id)
        supersedes_by_id[evidence_id] = supersedes
    for kind, by_generation in sorted(chains.items()):
        for generation, ids in sorted(by_generation.items()):
            if len(ids) != 1:
                errors.append(f"CONTROLLER_EVIDENCE_LINEAGE_FORK {label} {kind} "
                              f"generation={generation} ids={','.join(sorted(ids))}")
        if set(by_generation) != set(range(1, len(by_generation) + 1)):
            generations = ",".join(str(item) for item in sorted(by_generation))
            errors.append(f"CONTROLLER_EVIDENCE_LINEAGE_GAP {label} {kind} generations={generations}")
        line = {generation: ids[0] for generation, ids in by_generation.items() if len(ids) == 1}
        for generation, evidence_id in sorted(line.items()):
            expected = None if generation == 1 else line.get(generation - 1)
            if supersedes_by_id[evidence_id] != expected:
                errors.append(f"CONTROLLER_EVIDENCE_LINEAGE_ORDER {label}.evidence.{evidence_id} "
                              f"actual={supersedes_by_id[evidence_id]} expected={expected}")


def validate_evidence(record: dict[str, Any], at: datetime, where: str, errors: list[str],
                      *, normative: bool = True) -> None:
    strict_object(record, EVIDENCE_KEYS, where, errors, optional=EVIDENCE_OPTIONAL_KEYS)
    text(record.get("id"), f"{where}.id", errors, EVIDENCE_RE)
    if record.get("kind") not in {"merge_ci", "external_probe"}: errors.append(f"EVIDENCE_KIND {where}")
    repository_id = text(record.get("repository_id"), f"{where}.repository_id", errors, ID_RE)
    text(record.get("merge_sha"), f"{where}.merge_sha", errors, SHA_RE)
    text(record.get("content_digest"), f"{where}.content_digest", errors, DIGEST_RE)
    if record.get("provenance") != "measured": errors.append(f"EVIDENCE_NOT_MEASURED {where}")
    text(record.get("verifier"), f"{where}.verifier", errors, ID_RE)
    verified = timestamp(record.get("verified_at"), f"{where}.verified_at", errors)
    if verified is not None and verified > at: errors.append(f"EVIDENCE_FROM_FUTURE {where}")
    ci = strict_object(record.get("ci"), CI_KEYS, f"{where}.ci", errors)
    if ci.get("provider") != "github-actions" or ci.get("status") != "success": errors.append(f"EVIDENCE_CI_NOT_SUCCESS {where}")
    if not isinstance(ci.get("run_id"), str) or not ci.get("run_id", "").isdigit(): errors.append(f"EVIDENCE_CI_RUN_ID {where}")
    expected_url = f"{REPOSITORY_WEB.get(repository_id, '')}/actions/runs/{ci.get('run_id')}"
    if ci.get("url") != expected_url: errors.append(f"EVIDENCE_CI_URL {where}")
    if ci.get("head_sha") != record.get("merge_sha"): errors.append(f"EVIDENCE_CI_SHA_MISMATCH {where}")
    checks = string_list(ci.get("required_checks"), f"{where}.ci.required_checks", errors, nonempty=True)
    if normative and frozenset(checks) not in normative_check_sets(repository_id, record.get("merge_sha")):
        errors.append(f"EVIDENCE_REQUIRED_CHECKS_NONNORMATIVE {where}")
    contract = record.get("check_contract")
    expected_contract = CHECK_CONTRACT.get(repository_id)
    if expected_contract is None and contract is not None:
        errors.append(f"EVIDENCE_CHECK_CONTRACT_UNEXPECTED {where}")
    elif (normative and expected_contract is not None and contract is None
            and (repository_id, record.get("merge_sha")) not in HISTORICAL_UNCONTRACTED):
        errors.append(f"EVIDENCE_CHECK_CONTRACT_REQUIRED {where}")
    elif contract is not None:
        contract = strict_object(contract, CHECK_CONTRACT_KEYS, f"{where}.check_contract", errors)
        text(contract.get("path"), f"{where}.check_contract.path", errors)
        if (contract.get("path"), contract.get("format")) != expected_contract:
            errors.append(f"EVIDENCE_CHECK_CONTRACT_BINDING {where}")
        text(contract.get("content_digest"), f"{where}.check_contract.content_digest", errors, DIGEST_RE)
    if "lineage" in record:
        validate_evidence_lineage(record, where, errors)


def claim_write_paths(value: Any, where: str, errors: list[str]) -> list[str]:
    """Mirror the controller's canonical write-path syntax: exact paths or a `/**` scope."""
    paths = string_list(value, where, errors, nonempty=True)
    if paths != sorted(paths):
        errors.append(f"ORDER {where}")
    for index, path in enumerate(paths):
        scope = path[:-3] if path.endswith("/**") else path
        components = scope.split("/")
        if (path != path.strip()
                or path.startswith("/")
                or path.endswith("/")
                or "\\" in path
                or any(not part or part in {".", ".."} for part in components)
                or any(ord(character) < 32 or ord(character) == 127 for character in path)
                or any(marker in path for marker in "?[")
                or ("*" in path and (not path.endswith("/**") or "*" in path[:-3]))):
            errors.append(f"FORMAT {where}[{index}]")
    return paths


def paths_overlap(left: str, right: str) -> bool:
    """Return whether exact paths or trailing /** directory scopes intersect."""
    left_prefix = left[:-3].rstrip("/") if left.endswith("/**") else None
    right_prefix = right[:-3].rstrip("/") if right.endswith("/**") else None
    if left_prefix is None and right_prefix is None:
        return left == right
    if left_prefix is not None and right_prefix is not None:
        return (
            left_prefix == right_prefix
            or left_prefix.startswith(right_prefix + "/")
            or right_prefix.startswith(left_prefix + "/")
        )
    prefix, exact = (left_prefix, right) if left_prefix is not None else (right_prefix, left)
    return exact == prefix or exact.startswith(prefix + "/")


def claim_strings(record: dict[str, Any], key: str) -> set[str]:
    value = record.get(key)
    return {item for item in value if isinstance(item, str)} if isinstance(value, list) else set()


def claim_conflict_reasons(claim: dict[str, Any], other: dict[str, Any]) -> list[str]:
    """Mirror the controller's bounded-lane conflict rules for two active claims.

    A record without write_paths/content_scopes claims its whole repository, and the
    repository rule below already rejects any second active lane there, so a missing
    or malformed scope never widens what a concurrent lane is allowed to touch.
    """
    reasons: list[str] = []
    repository = claim.get("repository_id")
    same_repository = repository == other.get("repository_id")
    if same_repository:
        reasons.append(f"repository={repository}")
    if claim.get("work_item_id") == other.get("work_item_id"):
        reasons.append(f"work_item={claim.get('work_item_id')}")
    for capability in sorted(claim_strings(claim, "capabilities") & claim_strings(other, "capabilities")):
        reasons.append(f"capability={capability}")
    if same_repository:
        other_paths = claim_strings(other, "write_paths")
        for path in sorted(claim_strings(claim, "write_paths")):
            if any(paths_overlap(path, item) for item in other_paths):
                reasons.append(f"path={path}")
    for scope in sorted(claim_strings(claim, "content_scopes") & claim_strings(other, "content_scopes")):
        reasons.append(f"content_scope={scope}")
    return reasons


def validate_controller_claim(record: dict[str, Any], where: str, errors: list[str],
                              at: datetime | None = None) -> None:
    """Mirror the central delivery-schema claim semantics for every registry entry."""
    strict_object(record, CLAIM_KEYS, where, errors, optional=CLAIM_OPTIONAL_KEYS)
    text(record.get("id"), f"{where}.id", errors, ID_RE)
    text(record.get("work_item_id"), f"{where}.work_item_id", errors)
    text(record.get("owner_principal"), f"{where}.owner_principal", errors, ID_RE)
    text(record.get("repository_id"), f"{where}.repository_id", errors, ID_RE)
    text(record.get("base_sha"), f"{where}.base_sha", errors, SHA_RE)
    branch = text(record.get("branch"), f"{where}.branch", errors)
    if branch is not None and not valid_branch(branch):
        errors.append(f"INVALID_BRANCH {where}.branch")
    capabilities = string_list(record.get("capabilities"), f"{where}.capabilities", errors, nonempty=True)
    for index, capability in enumerate(capabilities):
        if not ID_RE.fullmatch(capability):
            errors.append(f"FORMAT {where}.capabilities[{index}]")
    generation = record.get("generation")
    if not isinstance(generation, int) or isinstance(generation, bool) or generation < 1:
        errors.append(f"FORMAT {where}.generation")
    if record.get("status") not in CLAIM_STATUSES:
        errors.append(f"FORMAT {where}.status")
    issued = timestamp(record.get("issued_at"), f"{where}.issued_at", errors)
    expires = timestamp(record.get("expires_at"), f"{where}.expires_at", errors)
    if issued is not None and expires is not None and issued >= expires:
        errors.append(f"CLAIM_TIME_ORDER {where}")
    if at is not None and issued is not None and expires is not None:
        if record.get("status") == "active" and not issued <= at < expires:
            errors.append(f"ACTIVE_CLAIM_TIME_INVALID {where}")
        if record.get("status") == "expired" and at < expires:
            errors.append(f"EXPIRED_CLAIM_TIME_INVALID {where}")
    refs = string_list(record.get("dependency_evidence_refs"), f"{where}.dependency_evidence_refs", errors)
    for index, evidence_ref in enumerate(refs):
        if not EVIDENCE_RE.fullmatch(evidence_ref):
            errors.append(f"FORMAT {where}.dependency_evidence_refs[{index}]")
    if "write_paths" in record:
        claim_write_paths(record.get("write_paths"), f"{where}.write_paths", errors)
    if "content_scopes" in record:
        scopes = string_list(record.get("content_scopes"), f"{where}.content_scopes", errors, nonempty=True)
        for index, scope in enumerate(scopes):
            if not ID_RE.fullmatch(scope):
                errors.append(f"FORMAT {where}.content_scopes[{index}]")


def required_evidence_refs(work: dict[str, Any], work_items: dict[str, dict[str, Any]],
                           externals: dict[str, dict[str, Any]], where: str,
                           errors: list[str]) -> set[str]:
    required: set[str] = set()
    for dependency_id in string_list(work.get("dependencies"), f"{where}.dependencies", errors):
        dependency = work_items.get(dependency_id)
        if dependency is None:
            errors.append(f"CONTROLLER_DEPENDENCY_MISSING {where} {dependency_id}")
        else:
            dependency_refs = dependency.get("evidence_refs")
            if isinstance(dependency_refs, list):
                required.update(item for item in dependency_refs if isinstance(item, str))
    for external_id in string_list(
        work.get("external_prerequisites"), f"{where}.external_prerequisites", errors
    ):
        external = externals.get(external_id)
        if external is None:
            errors.append(f"CONTROLLER_EXTERNAL_MISSING {where} {external_id}")
        else:
            external_refs = external.get("evidence_refs")
            if isinstance(external_refs, list):
                required.update(item for item in external_refs if isinstance(item, str))
    return required


def validate_controller_snapshot(documents: dict[str, dict[str, Any]], at: datetime,
                                 label: str, errors: list[str], *, claims_at: bool = False) -> dict[str, Any]:
    """Strictly validate one immutable controller claims/state/evidence snapshot."""
    claims_doc = strict_object(
        documents.get("claims.json"), {"document_type", "schema_version", "claims"},
        f"{label}.claims", errors,
    )
    state_doc = strict_object(
        documents.get("state.json"),
        {"document_type", "schema_version", "repositories", "capabilities",
         "external_capabilities", "work_items"},
        f"{label}.state", errors,
    )
    evidence_doc = strict_object(
        documents.get("evidence.json"), {"document_type", "schema_version", "evidence"},
        f"{label}.evidence", errors,
    )
    if claims_doc.get("document_type") != "claims-registry":
        errors.append(f"CONTROLLER_CLAIMS_TYPE {label}")
    if state_doc.get("document_type") != "delivery-state":
        errors.append(f"CONTROLLER_STATE_TYPE {label}")
    if evidence_doc.get("document_type") != "evidence-registry":
        errors.append(f"CONTROLLER_EVIDENCE_TYPE {label}")
    if not all(schema_version_one(document) for document in (claims_doc, state_doc, evidence_doc)):
        errors.append(f"CONTROLLER_SCHEMA_VERSION {label}")

    repositories = unique_index(state_doc.get("repositories"), f"{label}.repositories", errors)
    capabilities = unique_index(state_doc.get("capabilities"), f"{label}.capabilities", errors)
    externals = unique_index(
        state_doc.get("external_capabilities"), f"{label}.external_capabilities", errors
    )
    work_items = unique_index(state_doc.get("work_items"), f"{label}.work_items", errors)
    evidence = unique_index(evidence_doc.get("evidence"), f"{label}.evidence", errors)
    claims = unique_index(claims_doc.get("claims"), f"{label}.claims", errors)
    if set(repositories) != set(REPOSITORY_WEB):
        errors.append(f"CONTROLLER_REPOSITORIES_MISMATCH {label}")
    if not capabilities:
        errors.append(f"CONTROLLER_CAPABILITIES_EMPTY {label}")

    for repository_id, repository in repositories.items():
        where = f"{label}.repository.{repository_id}"
        strict_object(repository, CONTROLLER_REPOSITORY_KEYS, where, errors)
        text(repository.get("id"), f"{where}.id", errors, ID_RE)
        if repository.get("role") not in REPOSITORY_ROLES:
            errors.append(f"CONTROLLER_REPOSITORY_ROLE {where}")
        if repository.get("url") != REPOSITORY_WEB.get(repository_id):
            errors.append(f"CONTROLLER_REPOSITORY_URL {where}")
        text(repository.get("baseline_sha"), f"{where}.baseline_sha", errors, SHA_RE)

    for capability_id, capability in capabilities.items():
        where = f"{label}.capability.{capability_id}"
        strict_object(capability, CAPABILITY_KEYS, where, errors)
        text(capability.get("id"), f"{where}.id", errors, ID_RE)
        owner = text(capability.get("owner_repository"), f"{where}.owner_repository", errors, ID_RE)
        if owner not in repositories:
            errors.append(f"CONTROLLER_CAPABILITY_OWNER_MISSING {where}")
        if capability.get("write_exclusive") is not True:
            errors.append(f"CONTROLLER_CAPABILITY_NOT_EXCLUSIVE {where}")
        text(capability.get("scope"), f"{where}.scope", errors)

    for evidence_id, record in evidence.items():
        where = f"{label}.evidence.{evidence_id}"
        validate_evidence(record, at, where, errors, normative=False)
        if record.get("repository_id") not in repositories:
            errors.append(f"CONTROLLER_EVIDENCE_REPOSITORY_MISSING {where}")
    validate_evidence_lineage_chain(evidence, label, errors)

    for external_id, external in externals.items():
        where = f"{label}.external.{external_id}"
        strict_object(external, EXTERNAL_KEYS, where, errors)
        text(external.get("id"), f"{where}.id", errors, ID_RE)
        if external.get("state") not in EXTERNAL_STATES:
            errors.append(f"CONTROLLER_EXTERNAL_STATE {where}")
        refs = string_list(external.get("evidence_refs"), f"{where}.evidence_refs", errors)
        for evidence_id in refs:
            if not EVIDENCE_RE.fullmatch(evidence_id) or evidence_id not in evidence:
                errors.append(f"CONTROLLER_EVIDENCE_REF_MISSING {where} {evidence_id}")
        if external.get("state") == "available" and not refs:
            errors.append(f"CONTROLLER_EXTERNAL_EVIDENCE_MISSING {where}")
        string_list(external.get("blocks"), f"{where}.blocks", errors)

    for work_id, work in work_items.items():
        where = f"{label}.work.{work_id}"
        strict_object(work, WORK_KEYS, where, errors)
        if not re.fullmatch(r"^[A-Z]+(?:-[A-Z]+)+-[0-9]+$", str(work.get("id", ""))):
            errors.append(f"FORMAT {where}.id")
        owner = text(work.get("owner_repository"), f"{where}.owner_repository", errors, ID_RE)
        if owner not in repositories:
            errors.append(f"CONTROLLER_WORK_OWNER_MISSING {where}")
        work_capabilities = string_list(
            work.get("capabilities"), f"{where}.capabilities", errors, nonempty=True
        )
        for capability_id in work_capabilities:
            capability = capabilities.get(capability_id)
            if capability is None:
                errors.append(f"CONTROLLER_WORK_CAPABILITY_MISSING {where} {capability_id}")
            elif capability.get("owner_repository") != owner:
                errors.append(f"CONTROLLER_WORK_CAPABILITY_OWNER_MISMATCH {where} {capability_id}")
        if work.get("status") not in WORK_STATUSES:
            errors.append(f"CONTROLLER_WORK_STATUS {where}")
        work_evidence = string_list(work.get("evidence_refs"), f"{where}.evidence_refs", errors)
        for evidence_id in work_evidence:
            if evidence_id not in evidence:
                errors.append(f"CONTROLLER_EVIDENCE_REF_MISSING {where} {evidence_id}")
        if work.get("status") == "done" and not work_evidence:
            errors.append(f"CONTROLLER_DONE_WORK_EVIDENCE_MISSING {where}")
        # Resolve every reference even for non-ready work; terminal/unrelated claims cannot hide bad IDs.
        required_evidence_refs(work, work_items, externals, where, errors)
        if work.get("status") == "ready":
            dependencies = work.get("dependencies")
            unfinished = [dependency_id for dependency_id in dependencies
                          if isinstance(dependency_id, str)
                          and work_items.get(dependency_id, {}).get("status") != "done"] \
                if isinstance(dependencies, list) else []
            external_refs = work.get("external_prerequisites")
            unavailable = [external_id for external_id in external_refs
                           if isinstance(external_id, str)
                           and externals.get(external_id, {}).get("state") != "available"] \
                if isinstance(external_refs, list) else []
            if unfinished or unavailable:
                errors.append(f"CONTROLLER_WORK_NOT_READY {where}")

    for claim_id, claim in claims.items():
        where = f"{label}.claim.{claim_id}"
        validate_controller_claim(claim, where, errors, at if claims_at else None)
        repository_id = claim.get("repository_id")
        work = work_items.get(claim.get("work_item_id"))
        if repository_id not in repositories:
            errors.append(f"CONTROLLER_CLAIM_REPOSITORY_MISSING {where}")
        if work is None:
            errors.append(f"CONTROLLER_CLAIM_WORK_MISSING {where}")
            required: set[str] = set()
        else:
            if work.get("owner_repository") != repository_id:
                errors.append(f"CONTROLLER_CLAIM_WORK_OWNER_MISMATCH {where}")
            raw_claim_capabilities = claim.get("capabilities")
            claim_capabilities = ([item for item in raw_claim_capabilities if isinstance(item, str)]
                                  if isinstance(raw_claim_capabilities, list) else [])
            raw_work_capabilities = work.get("capabilities")
            work_capability_set = ({item for item in raw_work_capabilities if isinstance(item, str)}
                                   if isinstance(raw_work_capabilities, list) else set())
            if (not isinstance(raw_claim_capabilities, list)
                    or len(claim_capabilities) != len(raw_claim_capabilities)
                    or not set(claim_capabilities).issubset(work_capability_set)):
                errors.append(f"CONTROLLER_CLAIM_CAPABILITIES_NOT_SUBSET {where}")
            for capability_id in claim_capabilities:
                capability = capabilities.get(capability_id)
                if capability is None:
                    errors.append(f"CONTROLLER_CLAIM_CAPABILITY_MISSING {where} {capability_id}")
                elif capability.get("owner_repository") != repository_id:
                    errors.append(f"CONTROLLER_CLAIM_CAPABILITY_OWNER_MISMATCH {where} {capability_id}")
                elif capability.get("write_exclusive") is not True:
                    errors.append(f"CONTROLLER_CLAIM_CAPABILITY_NOT_EXCLUSIVE {where} {capability_id}")
            required = required_evidence_refs(work, work_items, externals, where, errors)
        raw_claim_refs = claim.get("dependency_evidence_refs")
        claim_refs = ({item for item in raw_claim_refs if isinstance(item, str)}
                      if isinstance(raw_claim_refs, list) else set())
        if not required.issubset(claim_refs):
            errors.append(f"CONTROLLER_CLAIM_REQUIRED_EVIDENCE_MISSING {where}")
        for evidence_id in claim_refs:
            if evidence_id not in evidence:
                errors.append(f"CONTROLLER_CLAIM_EVIDENCE_MISSING {where} {evidence_id}")

    return {
        "repositories": repositories, "capabilities": capabilities, "externals": externals,
        "work_items": work_items, "evidence": evidence, "claims": claims,
    }


def load_controller_snapshot(root: Path, revision: str, at: datetime, label: str,
                             errors: list[str], *, claims_at: bool = False) -> dict[str, Any]:
    documents = {
        name: git_json(root, revision, f"product/delivery/{name}", errors)
        for name in ("claims.json", "state.json", "evidence.json")
    }
    if any(not document for document in documents.values()):
        return {}
    return validate_controller_snapshot(documents, at, label, errors, claims_at=claims_at)


def issuance_prerequisite_errors(snapshot: dict[str, Any], claim: dict[str, Any],
                                 where: str) -> list[str]:
    """Prove that a claim was eligible using only its immutable issuance snapshot."""
    errors: list[str] = []
    work = snapshot["work_items"].get(claim.get("work_item_id"))
    if work is None:
        return [f"CONTROLLER_ISSUANCE_WORK_MISSING {where}"]
    if work.get("status") != "ready":
        errors.append(f"CONTROLLER_ISSUANCE_WORK_NOT_READY {where}")
    if work.get("owner_repository") != claim.get("repository_id"):
        errors.append(f"CONTROLLER_ISSUANCE_WORK_OWNER_MISMATCH {where}")
    if work.get("capabilities") != claim.get("capabilities"):
        errors.append(f"CONTROLLER_ISSUANCE_WORK_CAPABILITIES_MISMATCH {where}")

    required: set[str] = set()
    evidence = snapshot["evidence"]
    dependencies = work.get("dependencies")
    for dependency_id in dependencies if isinstance(dependencies, list) else []:
        if not isinstance(dependency_id, str):
            continue
        dependency = snapshot["work_items"].get(dependency_id)
        if dependency is None:
            errors.append(f"CONTROLLER_ISSUANCE_DEPENDENCY_MISSING {where} {dependency_id}")
            continue
        if dependency.get("status") != "done":
            errors.append(f"CONTROLLER_ISSUANCE_DEPENDENCY_NOT_DONE {where} {dependency_id}")
        dependency_owner = dependency.get("owner_repository")
        dependency_refs = dependency.get("evidence_refs")
        for evidence_id in dependency_refs if isinstance(dependency_refs, list) else []:
            if not isinstance(evidence_id, str):
                continue
            required.add(evidence_id)
            record = evidence.get(evidence_id)
            if record is None:
                continue
            if record.get("repository_id") != dependency_owner:
                errors.append(
                    f"CONTROLLER_ISSUANCE_DEPENDENCY_EVIDENCE_OWNER_MISMATCH "
                    f"{where} {dependency_id} {evidence_id}"
                )
            if record.get("kind") != "merge_ci":
                errors.append(
                    f"CONTROLLER_ISSUANCE_DEPENDENCY_EVIDENCE_TYPE_MISMATCH "
                    f"{where} {dependency_id} {evidence_id}"
                )

    external_prerequisites = work.get("external_prerequisites")
    for external_id in external_prerequisites if isinstance(external_prerequisites, list) else []:
        if not isinstance(external_id, str):
            continue
        external = snapshot["externals"].get(external_id)
        if external is None:
            errors.append(f"CONTROLLER_ISSUANCE_EXTERNAL_MISSING {where} {external_id}")
            continue
        if external.get("state") != "available":
            errors.append(f"CONTROLLER_ISSUANCE_EXTERNAL_UNAVAILABLE {where} {external_id}")
        # The external capability's exact evidence_refs are the central contract's explicit
        # cross-owner relation. Do not infer an owner from its ID or blocks list.
        external_refs = external.get("evidence_refs")
        for evidence_id in external_refs if isinstance(external_refs, list) else []:
            if not isinstance(evidence_id, str):
                continue
            required.add(evidence_id)
            record = evidence.get(evidence_id)
            if record is not None and record.get("kind") not in {"merge_ci", "external_probe"}:
                errors.append(
                    f"CONTROLLER_ISSUANCE_EXTERNAL_EVIDENCE_TYPE_MISMATCH "
                    f"{where} {external_id} {evidence_id}"
                )

    raw_claim_refs = claim.get("dependency_evidence_refs")
    claim_refs = ({item for item in raw_claim_refs if isinstance(item, str)}
                  if isinstance(raw_claim_refs, list) else set())
    if required != claim_refs:
        errors.append(f"CONTROLLER_ISSUANCE_REQUIRED_EVIDENCE_SET_MISMATCH {where}")
    issued = timestamp(claim.get("issued_at"), f"{where}.claim.issued_at", errors)
    for evidence_id in claim_refs:
        record = evidence.get(evidence_id)
        if record is None:
            continue
        verified = timestamp(
            record.get("verified_at"), f"{where}.evidence.{evidence_id}.verified_at", errors
        )
        if issued is not None and verified is not None and verified > issued:
            errors.append(f"CONTROLLER_ISSUANCE_EVIDENCE_AFTER_CLAIM {where} {evidence_id}")
    return errors


def admission_claim_identity(admission: dict[str, Any]) -> dict[str, Any]:
    """Immutable claim fields an execution state binds itself to, status excluded.

    `claim_digest` is defined over this identity plus `status: active`, never over the
    whole registry record: a record may additionally carry bounded-lane scoping fields
    that the admission document does not bind.
    """
    return {
        "id": admission["claim_id"], "work_item_id": admission["work_item_id"],
        "owner_principal": admission["owner_principal"], "repository_id": "ccm-public",
        "base_sha": admission["base_sha"], "branch": admission["branch"],
        "capabilities": admission["capabilities"], "generation": admission["claim_generation"],
        "issued_at": admission["issued_at"], "expires_at": admission["expires_at"],
        "dependency_evidence_refs": admission["dependency_evidence_refs"],
    }


def binds_active_claim(claim: dict[str, Any], identity: dict[str, Any]) -> bool:
    """Whether one registry record is the still-active claim the admission was issued for."""
    return (claim.get("status") == "active"
            and all(claim.get(key) == value for key, value in identity.items()))


def admission_snapshot_errors(controller_root: Path, controller_head: str, public_root: Path,
                              state: dict[str, Any], at: datetime, where: str) -> list[str]:
    """Authenticate one execution state against its own immutable issuance snapshot."""
    errors: list[str] = []
    admission = state["admission"]
    issuance = admission["controller_commit_sha"]
    if git(controller_root, "cat-file", "-e", f"{issuance}^{{commit}}", check=False).returncode:
        return [f"CONTROLLER_ISSUANCE_UNAVAILABLE {where}"]
    if not is_ancestor(controller_root, issuance, controller_head):
        errors.append(f"CONTROLLER_ISSUANCE_NOT_ANCESTOR {where}")
    snapshot = load_controller_snapshot(
        controller_root, issuance, at, f"{where}.issuance", errors
    )
    if not snapshot:
        return errors
    claim = snapshot["claims"].get(admission["claim_id"])
    if claim is None:
        return errors + [f"CONTROLLER_ISSUANCE_CLAIM_MISSING {where}"]
    identity = admission_claim_identity(admission)
    if not binds_active_claim(claim, identity):
        errors.append(f"CONTROLLER_ISSUANCE_CLAIM_BINDING_MISMATCH {where}")
    if canonical_digest({**identity, "status": "active"}) != admission["claim_digest"]:
        errors.append(f"CONTROLLER_ISSUANCE_CLAIM_DIGEST_MISMATCH {where}")
    errors.extend(issuance_prerequisite_errors(snapshot, claim, where))
    bindings = {item["id"]: item["digest"] for item in admission["dependency_evidence"]}
    for evidence_id in admission["dependency_evidence_refs"]:
        record = snapshot["evidence"].get(evidence_id)
        if record is None:
            errors.append(f"CONTROLLER_ISSUANCE_EVIDENCE_MISSING {where} {evidence_id}")
            continue
        validate_evidence(record, at, f"{where}.issuance.evidence.{evidence_id}", errors)
        if canonical_digest(record) != bindings.get(evidence_id):
            errors.append(f"CONTROLLER_ISSUANCE_EVIDENCE_DIGEST_MISMATCH {where} {evidence_id}")
        contract = record.get("check_contract")
        if isinstance(contract, dict):
            if record.get("repository_id") != "ccm-public":
                errors.append(f"EVIDENCE_CHECK_CONTRACT_OWNER_UNAVAILABLE {where} {evidence_id}")
                continue
            public_head = git(public_root, "rev-parse", "HEAD", check=False).stdout.strip()
            if not is_ancestor(public_root, record.get("merge_sha", ""), public_head):
                errors.append(f"EVIDENCE_CHECK_CONTRACT_COMMIT_UNREACHABLE {where} {evidence_id}")
            content = git_bytes(
                public_root, "show", f"{record.get('merge_sha')}:{contract.get('path')}"
            )
            measured = bytes_digest(content.stdout) if content.returncode == 0 else None
            if measured != contract.get("content_digest"):
                errors.append(f"EVIDENCE_CHECK_CONTRACT_DIGEST_MISMATCH {where} {evidence_id}")
    return errors


def analyze_claim_history(claims: dict[str, dict[str, Any]], claim: dict[str, Any],
                          predecessor: dict[str, Any] | None) -> tuple[list[str], list[dict[str, Any]]]:
    errors: list[str] = []
    work_item = claim.get("work_item_id")
    repository = claim.get("repository_id")
    capabilities = claim.get("capabilities")
    capability_set = (frozenset(capabilities) if isinstance(capabilities, list)
                      and all(isinstance(item, str) for item in capabilities) else frozenset())
    same_owner = [item for item in claims.values()
                  if item.get("work_item_id") == work_item and item.get("repository_id") == repository]
    lineage: list[dict[str, Any]] = []
    for item in same_owner:
        item_capabilities = item.get("capabilities")
        if (not isinstance(item_capabilities, list)
                or not all(isinstance(value, str) for value in item_capabilities)
                or len(item_capabilities) != len(set(item_capabilities))
                or frozenset(item_capabilities) != capability_set):
            errors.append(f"CONTROLLER_CLAIM_LINEAGE_CAPABILITIES_MISMATCH {item.get('id', '<unknown>')}")
        else:
            lineage.append(item)
    by_generation: dict[int, list[dict[str, Any]]] = {}
    for item in lineage:
        generation = item.get("generation")
        if not isinstance(generation, int) or isinstance(generation, bool) or generation < 1:
            errors.append(f"CONTROLLER_CLAIM_HISTORY_GENERATION_INVALID {item.get('id', '<unknown>')}")
            continue
        by_generation.setdefault(generation, []).append(item)
    for generation, items in sorted(by_generation.items()):
        if len(items) != 1:
            errors.append(f"CONTROLLER_CLAIM_GENERATION_FORK {generation}")
    current_generation = claim.get("generation")
    if isinstance(current_generation, int) and not isinstance(current_generation, bool) and current_generation >= 1:
        for generation in range(1, current_generation + 1):
            if len(by_generation.get(generation, [])) != 1:
                errors.append(f"CONTROLLER_CLAIM_HISTORY_INCOMPLETE {generation}")
        current = by_generation.get(current_generation, [])
        if len(current) == 1 and current[0].get("id") != claim.get("id"):
            errors.append("CONTROLLER_CURRENT_CLAIM_GENERATION_MISMATCH")
        if current_generation > 1 and predecessor is not None:
            previous = by_generation.get(current_generation - 1, [])
            if len(previous) == 1 and predecessor.get("claim_id") != previous[0].get("id"):
                errors.append("PREDECESSOR_CLAIM_ID_MISMATCH")
        for generation in range(1, current_generation):
            previous = by_generation.get(generation, [])
            if len(previous) == 1 and previous[0].get("status") not in TERMINAL_CLAIM_STATUSES:
                errors.append(f"CONTROLLER_PREDECESSOR_NOT_TERMINAL {generation}")
    ordered: list[dict[str, Any]] = []
    if isinstance(current_generation, int) and not isinstance(current_generation, bool):
        ordered = [by_generation[generation][0] for generation in range(1, current_generation + 1)
                   if len(by_generation.get(generation, [])) == 1]
    return errors, ordered


def claim_history_errors(claims: dict[str, dict[str, Any]], claim: dict[str, Any],
                         predecessor: dict[str, Any] | None) -> list[str]:
    return analyze_claim_history(claims, claim, predecessor)[0]


def state_claim_identity_errors(state: dict[str, Any], claim: dict[str, Any],
                                where: str) -> list[str]:
    errors: list[str] = []
    admission = state["admission"]
    identity = admission_claim_identity(admission)
    if any(claim.get(key) != value for key, value in identity.items()):
        errors.append(f"LINEAGE_STATE_CLAIM_IDENTITY_MISMATCH {where}")
    if canonical_digest({**identity, "status": "active"}) != admission["claim_digest"]:
        errors.append(f"LINEAGE_STATE_ACTIVE_CLAIM_DIGEST_MISMATCH {where}")
    return errors


def state_at_checkpoint(root: Path, checkpoint: str, claim_id: str,
                        errors: list[str]) -> dict[str, Any] | None:
    path = Path("delivery/executions") / f"{claim_id}.json"
    tree = git(root, "ls-tree", checkpoint, "--", path.as_posix(), check=False)
    lines = tree.stdout.splitlines()
    if len(lines) != 1 or not lines[0].startswith("100644 blob "):
        errors.append(f"LINEAGE_STATE_MODE_OR_ENTRY_MISMATCH {claim_id}")
        return None
    state = previous_state(root, checkpoint, path)
    if state is None:
        errors.append(f"LINEAGE_STATE_MISSING_OR_INVALID {claim_id}")
    return state


def lineage_state_errors(public_root: Path, controller_root: Path, controller_head: str,
                         state: dict[str, Any], lineage: list[dict[str, Any]],
                         at: datetime) -> list[str]:
    """Bind every generation to its exact state path and recursively named checkpoint."""
    errors: list[str] = []
    if not lineage:
        return ["CONTROLLER_CLAIM_LINEAGE_UNAVAILABLE"]
    current_generation = state["admission"]["claim_generation"]
    by_generation = {claim["generation"]: claim for claim in lineage
                     if isinstance(claim.get("generation"), int)}
    current_claim = by_generation.get(current_generation)
    if current_claim is None:
        return ["CONTROLLER_CURRENT_CLAIM_GENERATION_MISSING"]
    errors.extend(state_claim_identity_errors(state, current_claim, f"generation-{current_generation}"))
    errors.extend(admission_snapshot_errors(
        controller_root, controller_head, public_root, state, at,
        f"generation-{current_generation}",
    ))
    child_state = state
    for generation in range(current_generation - 1, 0, -1):
        link = child_state["admission"].get("predecessor")
        claim = by_generation.get(generation)
        if claim is None or not isinstance(link, dict):
            errors.append(f"LINEAGE_PREDECESSOR_LINK_MISSING {generation}")
            break
        if link.get("claim_id") != claim.get("id") or link.get("generation") != generation:
            errors.append(f"LINEAGE_PREDECESSOR_LINK_MISMATCH {generation}")
            break
        previous = state_at_checkpoint(public_root, link.get("checkpoint_sha", ""), claim["id"], errors)
        if previous is None:
            break
        errors.extend(state_claim_identity_errors(previous, claim, f"generation-{generation}"))
        errors.extend(admission_snapshot_errors(
            controller_root, controller_head, public_root, previous, at, f"generation-{generation}"
        ))
        errors.extend(append_only_errors(previous, child_state))
        child_state = previous
    if current_generation == 1:
        child_state = state
    if child_state["admission"].get("claim_generation") == 1 and child_state["admission"].get("predecessor") is not None:
        errors.append("LINEAGE_GENERATION_ONE_HAS_PREDECESSOR")
    return errors


def controller_errors(root: Path, head: str, state: dict[str, Any], at: datetime,
                      public_root: Path) -> tuple[list[str], bool, list[str]]:
    errors = repository_guard(root, "ccm-multi")
    unavailable = False
    if errors: return errors, True, []
    errors.extend(remote_errors(root, CONTROLLER_REMOTE))
    admission = state["admission"]
    if not SHA_RE.fullmatch(head) or git(root, "cat-file", "-e", f"{head}^{{commit}}", check=False).returncode:
        return errors + ["CONTROLLER_HEAD_UNAVAILABLE"], True, []
    remote_main = git(root, "rev-parse", "refs/remotes/origin/main", check=False)
    if remote_main.returncode or remote_main.stdout.strip() != head:
        return errors + ["CONTROLLER_PREFETCHED_ORIGIN_MAIN_MISMATCH"], True, []
    issuance = admission["controller_commit_sha"]
    if git(root, "cat-file", "-e", f"{issuance}^{{commit}}", check=False).returncode:
        return errors + ["CONTROLLER_ISSUANCE_UNAVAILABLE"], True, []
    if not is_ancestor(root, issuance, head): errors.append("CONTROLLER_ISSUANCE_NOT_ANCESTOR")
    snapshot = load_controller_snapshot(root, head, at, "controller", errors, claims_at=True)
    if not snapshot:
        return errors, True, []
    claims = snapshot["claims"]
    work_items = snapshot["work_items"]
    externals = snapshot["externals"]
    evidence = snapshot["evidence"]
    claim_id = admission["claim_id"]
    claim = claims.get(claim_id)
    if claim is None:
        errors.append("CONTROLLER_CLAIM_MISSING")
        return errors, False, []
    history_errors, lineage = analyze_claim_history(claims, claim, admission["predecessor"])
    errors.extend(history_errors)
    identity = admission_claim_identity(admission)
    if canonical_digest({**identity, "status": "active"}) != admission["claim_digest"]:
        errors.append("CLAIM_DIGEST_MISMATCH")
    if not binds_active_claim(claim, identity):
        errors.extend(["CONTROLLER_CLAIM_REVOKED_OR_CHANGED", "CLAIM_BINDING_MISMATCH"])
    claim_issued = timestamp(claim.get("issued_at"), "controller.claim.issued_at", errors)
    claim_expires = timestamp(claim.get("expires_at"), "controller.claim.expires_at", errors)
    if claim_issued is not None and claim_expires is not None and not claim_issued <= at < claim_expires:
        errors.append("CONTROLLER_CLAIM_EXPIRED_OR_NOT_STARTED")
    # Bounded lanes: this claim must be the active one for its own scope, and every other
    # active lane must stay disjoint from it under the controller's own conflict rules.
    if claim.get("status") != "active":
        errors.append("CONTROLLER_ACTIVE_CLAIM_CONFLICT")
    for other_id, other in claims.items():
        if other_id == claim_id or other.get("status") != "active":
            continue
        reasons = claim_conflict_reasons(claim, other)
        if reasons:
            errors.append(f"CONTROLLER_ACTIVE_CLAIM_CONFLICT {other_id} {','.join(reasons)}")
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
        if required != set(admission["dependency_evidence_refs"]): errors.append("CLAIM_REQUIRED_EVIDENCE_SET_MISMATCH")
    bindings = {item["id"]: item["digest"] for item in admission["dependency_evidence"]}
    for evidence_id in admission["dependency_evidence_refs"]:
        record = evidence.get(evidence_id)
        if record is None:
            errors.append(f"CONTROLLER_EVIDENCE_MISSING {evidence_id}")
            continue
        validate_evidence(record, at, f"controller.evidence.{evidence_id}", errors)
        if canonical_digest(record) != bindings.get(evidence_id): errors.append(f"EVIDENCE_DIGEST_MISMATCH {evidence_id}")
        contract = record.get("check_contract")
        if isinstance(contract, dict):
            if record.get("repository_id") != "ccm-public":
                errors.append(f"EVIDENCE_CHECK_CONTRACT_OWNER_UNAVAILABLE {evidence_id}")
            else:
                if not is_ancestor(public_root, record.get("merge_sha", ""), git(public_root, "rev-parse", "HEAD").stdout.strip()):
                    errors.append(f"EVIDENCE_CHECK_CONTRACT_COMMIT_UNREACHABLE {evidence_id}")
                content = git_bytes(public_root, "show", f"{record.get('merge_sha')}:{contract.get('path')}")
                measured = "sha256:" + hashlib.sha256(content.stdout).hexdigest() if content.returncode == 0 else None
                if measured != contract.get("content_digest"):
                    errors.append(f"EVIDENCE_CHECK_CONTRACT_DIGEST_MISMATCH {evidence_id}")
    predecessor = admission["predecessor"]
    if predecessor:
        previous_claim = claims.get(predecessor["claim_id"])
        if previous_claim is None or previous_claim.get("generation") != predecessor["generation"]:
            errors.append("PREDECESSOR_CLAIM_MISSING")
        elif previous_claim.get("status") not in TERMINAL_CLAIM_STATUSES or previous_claim.get("work_item_id") != claim.get("work_item_id") or previous_claim.get("repository_id") != "ccm-public":
            errors.append("PREDECESSOR_CLAIM_NOT_CLOSED_OR_MISMATCHED")
        else:
            predecessor_path = Path("delivery/executions") / f"{predecessor['claim_id']}.json"
            predecessor_state = previous_state(public_root, predecessor["checkpoint_sha"], predecessor_path)
            if predecessor_state is None:
                errors.append("PREDECESSOR_ADMISSION_STATE_MISSING")
            else:
                prior_admission = predecessor_state["admission"]
                prior_identity = admission_claim_identity(prior_admission)
                if any(previous_claim.get(key) != value for key, value in prior_identity.items()):
                    errors.append("PREDECESSOR_ADMISSION_IDENTITY_MISMATCH")
                if canonical_digest({**prior_identity, "status": "active"}) != prior_admission["claim_digest"]:
                    errors.append("PREDECESSOR_ACTIVE_CLAIM_DIGEST_MISMATCH")
    errors.extend(lineage_state_errors(public_root, root, head, state, lineage, at))
    return errors, unavailable, [claim["id"] for claim in lineage]


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
    try:
        target = root / path
        if target.resolve(strict=True) != Path(os.path.abspath(target)) or not stat.S_ISREG(target.lstat().st_mode) or target.is_symlink():
            errors.append("STATE_WORKTREE_PATH_UNSAFE")
        worktree = target.read_bytes()
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


def task_acceptance(raw: bytes, work_item: str, errors: list[str]) -> list[str]:
    try: lines = raw.decode("utf-8").splitlines()
    except UnicodeDecodeError:
        errors.append("TODO_NOT_UTF8"); return []
    heading = f"### {work_item}"
    try: start = lines.index(heading) + 1
    except ValueError:
        errors.append("TODO_ACCEPTANCE_HEADING_MISSING"); return []
    selected: list[str] = []
    for line in lines[start:]:
        if line.startswith("### "): break
        selected.append(line)
    paragraphs: list[str] = []
    current: list[str] = []
    for line in selected + [""]:
        stripped = line.strip()
        if stripped: current.append(stripped)
        elif current: paragraphs.append(" ".join(current)); current = []
    if not paragraphs: errors.append("TODO_ACCEPTANCE_EMPTY")
    return paragraphs


def scope_allows(path: str, scopes: list[str], state_paths: set[str]) -> bool:
    if path in state_paths:
        return True
    return any(path == scope.rstrip("/") or (scope.endswith("/") and path.startswith(scope)) for scope in scopes)


def public_contract_errors(root: Path, head: str, state_path: Path, state: dict[str, Any],
                           inspector_digest: str,
                           lineage_claim_ids: list[str] | None = None) -> list[str]:
    errors: list[str] = []
    admission = state["admission"]
    base = admission["base_sha"]
    manifest_raw = git_bytes(root, "show", f"{base}:delivery/capabilities.json")
    todo = git_bytes(root, "show", f"{base}:TODO.md")
    if manifest_raw.returncode:
        errors.append("BASE_MANIFEST_UNAVAILABLE"); return errors
    if todo.returncode:
        errors.append("BASE_TODO_UNAVAILABLE"); return errors
    manifest = load_json_bytes(manifest_raw.stdout, f"{base}:delivery/capabilities.json", errors)
    matches = [item for item in manifest.get("capabilities", []) if isinstance(item, dict)
               and item.get("id") == admission["public_capability_id"]
               and item.get("work_item") == admission["work_item_id"]]
    if len(matches) != 1 or matches[0].get("status") != "ready":
        return errors + ["PUBLIC_CAPABILITY_NOT_READY_OR_MISMATCHED"]
    capability = matches[0]
    checks = capability.get("required_checks")
    if not isinstance(checks, list) or not checks or len(checks) != len(set(checks)) or not all(isinstance(item, str) and item for item in checks):
        errors.append("PUBLIC_REQUIRED_CHECKS_INVALID")
        checks = []
    elif checks != admission["required_checks"]:
        errors.append("PUBLIC_REQUIRED_CHECKS_BINDING_MISMATCH")
    expected_spec = f"TODO.md#{admission['work_item_id'].lower()}"
    if capability.get("specification") != expected_spec: errors.append("PUBLIC_SPECIFICATION_BINDING_MISMATCH")
    acceptance = task_acceptance(todo.stdout, admission["work_item_id"], errors)
    if canonical_digest(acceptance) != admission["acceptance_digest"]: errors.append("ACCEPTANCE_DIGEST_MISMATCH")
    scopes = capability.get("content_scope")
    if not isinstance(scopes, list) or not scopes or not all(isinstance(item, str) and item and not item.startswith(("/", ".")) and ".." not in Path(item).parts for item in scopes):
        errors.append("PUBLIC_CONTENT_SCOPE_INVALID"); scopes = []
    contract = {
        "manifest_digest": bytes_digest(manifest_raw.stdout),
        "todo_digest": bytes_digest(todo.stdout),
        "acceptance": acceptance,
        "capability": capability,
    }
    if canonical_digest(contract) != admission["contract_digest"]: errors.append("CONTRACT_DIGEST_MISMATCH")
    state_paths = {state_path.as_posix()}
    for claim_id in lineage_claim_ids or []:
        state_paths.add(f"delivery/executions/{claim_id}.json")
    changed = git_bytes(root, "diff", "--no-renames", "--name-only", "-z", base, head)
    if changed.returncode:
        errors.append("CHANGED_PATHS_UNAVAILABLE")
    else:
        for raw_path in changed.stdout.split(b"\0"):
            if not raw_path: continue
            try: path = raw_path.decode("utf-8")
            except UnicodeDecodeError:
                errors.append("CHANGED_PATH_NOT_UTF8"); continue
            if not scope_allows(path, scopes, state_paths): errors.append(f"CONTENT_SCOPE_VIOLATION {path}")
    inspector_path = Path(__file__)
    base_inspector = git_bytes(root, "show", f"{base}:.agents/skills/ccm-delivery-executor/scripts/inspect_state.py")
    if base_inspector.returncode:
        errors.append("BASE_INSPECTOR_UNAVAILABLE")
    elif bytes_digest(base_inspector.stdout) != inspector_digest:
        errors.append("BASE_INSPECTOR_DIGEST_MISMATCH")
    try: running_digest = bytes_digest(inspector_path.read_bytes())
    except OSError:
        errors.append("INSPECTOR_UNREADABLE")
    else:
        if inspector_digest != admission["inspector_digest"] or running_digest != inspector_digest:
            errors.append("INSPECTOR_EXTERNAL_DIGEST_MISMATCH")
    completed = state["execution"]["completed_acceptance"]
    if completed != acceptance[:len(completed)]: errors.append("COMPLETED_ACCEPTANCE_NOT_NORMATIVE_PREFIX")
    completed_checks = state["execution"]["completed_checks"]
    if any(item.get("name") not in checks for item in completed_checks if isinstance(item, dict)):
        errors.append("COMPLETED_CHECK_NOT_NORMATIVE")
    if state["execution"]["phase"] == "candidate":
        if completed != acceptance: errors.append("CANDIDATE_ACCEPTANCE_INCOMPLETE")
        if [item.get("name") for item in completed_checks] != checks:
            errors.append("CANDIDATE_REQUIRED_CHECKS_INCOMPLETE")
    return errors


def inspect(root: Path, state_path: Path, state: dict[str, Any], at: datetime, remote_head: str,
            controller_root: Path | None = None, controller_head: str | None = None,
            public_main_head: str | None = None, predecessor_remote_head: str | None = None,
            inspector_digest: str | None = None) -> dict[str, Any]:
    errors = validate_state(state)
    result: dict[str, Any] = {"admitted": False, "classification": "invalid", "errors": errors,
                              "facts": {}, "next_action": state.get("checkpoint", {}).get("next_action")}
    if errors: return result
    if not SHA_RE.fullmatch(remote_head):
        result["errors"].append("FORMAT remote_head")
        return result
    guard_errors = repository_guard(root)
    if guard_errors:
        result["errors"].extend(guard_errors)
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
    if public_main_head is None or not SHA_RE.fullmatch(public_main_head):
        result["errors"].append("PUBLIC_MAIN_MEASUREMENT_REQUIRED"); return result
    if inspector_digest is None or not DIGEST_RE.fullmatch(inspector_digest):
        result["errors"].append("INSPECTOR_EXTERNAL_DIGEST_REQUIRED"); return result
    prefetched_main = git(root, "rev-parse", "refs/remotes/origin/main", check=False)
    if prefetched_main.returncode:
        result["errors"].append("PUBLIC_PREFETCHED_ORIGIN_MAIN_MISSING"); return result
    if git(root, "cat-file", "-e", f"{public_main_head}^{{commit}}", check=False).returncode:
        result["errors"].append("PUBLIC_MAIN_MEASURED_HEAD_UNKNOWN"); return result
    if prefetched_main.stdout.strip() != public_main_head:
        result["errors"].append("PUBLIC_PREFETCHED_ORIGIN_MAIN_MISMATCH"); return result
    if public_main_head != admission["base_sha"]:
        result["errors"].append("PUBLIC_MAIN_BASE_MISMATCH"); return result
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
                if predecessor_remote_head is None or not SHA_RE.fullmatch(predecessor_remote_head):
                    stale.append("PREDECESSOR_REMOTE_HEAD_REQUIRED")
                elif git(root, "cat-file", "-e", f"{predecessor_remote_head}^{{commit}}", check=False).returncode:
                    stale.append("PREDECESSOR_REMOTE_HEAD_UNKNOWN")
                elif predecessor_remote_head != predecessor["checkpoint_sha"]:
                    stale.append("PREDECESSOR_NOT_LATEST_REMOTE_HEAD")
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
    if controller_root is None or controller_head is None:
        result["classification"] = "local_only"; result["errors"].append("CONTROLLER_SNAPSHOT_REQUIRED"); return result
    if not controller_root.is_dir():
        result["classification"] = "local_only"; result["errors"].append("CONTROLLER_ROOT_UNAVAILABLE"); return result
    canonical_controller_lexical = root.parent / "ccm-multi"
    try:
        canonical_controller = canonical_controller_lexical.resolve(strict=True)
        supplied_controller = controller_root.resolve(strict=True)
    except OSError:
        result["classification"] = "local_only"; result["errors"].append("CONTROLLER_ROOT_UNAVAILABLE"); return result
    if (controller_root != canonical_controller_lexical or supplied_controller != controller_root
            or supplied_controller != canonical_controller):
        result["classification"] = "local_only"; result["errors"].append("CONTROLLER_ROOT_NONCANONICAL"); return result
    control_errors, unavailable, lineage_claim_ids = controller_errors(
        controller_root, controller_head, state, at, root
    )
    if control_errors:
        result["classification"] = "local_only" if unavailable else "invalid"
        result["errors"].extend(control_errors)
        return result
    contract_errors = public_contract_errors(
        root, head, state_path, state, inspector_digest, lineage_claim_ids
    )
    if contract_errors:
        result["errors"].extend(contract_errors)
        return result
    result["classification"], result["admitted"] = "clean", True
    return result


def parse_now(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None or parsed.utcoffset() is None: raise ValueError("--at must include a timezone")
    return parsed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    commands = parser.add_subparsers(dest="command", required=True)
    validate_parser = commands.add_parser("validate", allow_abbrev=False)
    validate_parser.add_argument("--state", required=True, type=Path)
    inspect_parser = commands.add_parser("inspect", allow_abbrev=False)
    inspect_parser.add_argument("--root", required=True, type=Path)
    inspect_parser.add_argument("--state", required=True, type=Path)
    inspect_parser.add_argument("--at", required=True)
    inspect_parser.add_argument("--remote-head", required=True)
    inspect_parser.add_argument("--public-main-head", required=True)
    inspect_parser.add_argument("--predecessor-remote-head")
    inspect_parser.add_argument("--inspector-digest", required=True)
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
    root = Path(os.path.abspath(args.root))
    if root != root.resolve():
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": ["REPOSITORY_ROOT_NOT_CANONICAL"]}, sort_keys=True)); return 1
    try:
        lexical_state = Path(os.path.abspath(state_path))
        if lexical_state != lexical_state.resolve(): raise ValueError
        relative_state = lexical_state.relative_to(root)
    except ValueError:
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": ["STATE_OUTSIDE_ROOT"]}, sort_keys=True)); return 1
    result = inspect(root, relative_state, state, at, args.remote_head,
                     Path(os.path.abspath(args.controller_root)) if args.controller_root else None, args.controller_head,
                     args.public_main_head, args.predecessor_remote_head, args.inspector_digest)
    print(json.dumps(result, sort_keys=True))
    return 0 if result["admitted"] else 1


if __name__ == "__main__": sys.exit(main())
