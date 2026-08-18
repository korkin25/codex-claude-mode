#!/usr/bin/env python3
"""Authenticate delivery-report provider and Git references before rendering."""

from __future__ import annotations

import copy
from collections.abc import Iterator
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time
from typing import Any, BinaryIO, cast

import validate_report as renderer


GH_VERSION = (2, 97, 0)
OUTPUT_LIMIT = 2_000_000
COMMAND_TIMEOUT = 30.0
INPUT_LIMIT = 131_072
MAX_NESTING_DEPTH = 64
GIT_CANDIDATES = tuple(map(Path, (
    "/usr/bin/git", "/bin/git", "/usr/local/bin/git", "/opt/homebrew/bin/git",
)))
GH_CANDIDATES = tuple(map(Path, (
    "/usr/bin/gh", "/usr/local/bin/gh", "/opt/homebrew/bin/gh",
)))
SSH_CANDIDATES = tuple(map(Path, (
    "/usr/bin/ssh", "/bin/ssh", "/usr/local/bin/ssh", "/opt/homebrew/bin/ssh",
)))
FALSE_CANDIDATES = (Path("/usr/bin/false"), Path("/bin/false"))
TEMP_BASE_CANDIDATES = (Path("/tmp"), Path("/private/tmp"))
SKILL_REPOSITORY_ROOT = Path(__file__).resolve().parents[4]
RAW_POSTMERGE_KEYS = renderer.EVENT_KEYS["postmerge"] - {"files"}


class VerificationError(ValueError):
    pass


def _consistent_system_owner(nodes: list[os.stat_result], effective_uid: int) -> bool:
    if not nodes:
        return False
    system_uid = nodes[0].st_uid
    return (
        (effective_uid == 0 or system_uid != effective_uid)
        and all(node.st_uid == system_uid for node in nodes)
    )


def _terminate(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except OSError:
        if process.poll() is None:
            try:
                process.kill()
            except ProcessLookupError:
                pass
    process.wait()


def _completed(command: list[str], *, cwd: Path | None, env: dict[str, str],
               strip: bool = True, timeout_seconds: float = COMMAND_TIMEOUT,
               output_limit: int = OUTPUT_LIMIT) -> str:
    process: subprocess.Popen[bytes] | None = None
    try:
        process = subprocess.Popen(
            command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
        )
    except OSError as exc:
        raise VerificationError(f"command unavailable: {command[0]}") from exc
    assert process.stdout is not None and process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, ("stdout", process.stdout))
    selector.register(process.stderr, selectors.EVENT_READ, ("stderr", process.stderr))
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    deadline = time.monotonic() + timeout_seconds
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                _terminate(process)
                raise VerificationError(f"command timeout: {command[0]}")
            events = selector.select(remaining)
            if not events:
                _terminate(process)
                raise VerificationError(f"command timeout: {command[0]}")
            for key, _ in events:
                label, registered = key.data
                stream = cast(BinaryIO, registered)
                chunk = os.read(stream.fileno(), min(65_536, output_limit - len(buffers[label]) + 1))
                if not chunk:
                    selector.unregister(stream)
                    stream.close()
                    continue
                buffers[label].extend(chunk)
                if len(buffers[label]) > output_limit:
                    _terminate(process)
                    raise VerificationError(f"command output limit: {command[0]}")
        returncode = process.wait()
    finally:
        selector.close()
        for stream in (process.stdout, process.stderr):
            if not stream.closed:
                stream.close()
    if returncode:
        detail = bytes(buffers["stderr"]).decode("utf-8", "replace").strip()[:300]
        raise VerificationError(f"command failed: {command[0]}: {detail}")
    decoded = bytes(buffers["stdout"]).decode("utf-8", "strict")
    return decoded.strip() if strip else decoded


def _trusted_executable(candidates: tuple[Path, ...], label: str) -> Path:
    effective_uid = os.geteuid()
    for candidate in candidates:
        try:
            resolved = candidate.resolve(strict=True)
            leaf = resolved.lstat()
            parents = [parent.lstat() for parent in resolved.parents]
        except OSError:
            continue
        nodes = [leaf, *parents]
        system_uid = leaf.st_uid
        if (
            not stat.S_ISREG(leaf.st_mode)
            or not _consistent_system_owner(nodes, effective_uid)
            or not leaf.st_mode & 0o111
            or leaf.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
        ):
            continue
        if any(
            not stat.S_ISDIR(item.st_mode)
            or item.st_uid != system_uid
            or item.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
            for item in nodes[1:]
        ):
            continue
        return resolved
    raise VerificationError(f"trusted {label} executable unavailable")


def _is_within(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
    except ValueError:
        return False
    return True


def _caller_path(value: str | None) -> Path | None:
    if not value:
        return None
    try:
        return Path(os.path.abspath(value)).resolve(strict=False)
    except OSError:
        return None


def _trusted_temp_base(protected_paths: tuple[Path, ...]) -> Path:
    """Select a fixed system temp root without consulting caller temp variables."""
    effective_uid = os.geteuid()
    for candidate in TEMP_BASE_CANDIDATES:
        try:
            resolved = candidate.resolve(strict=True)
            leaf = resolved.lstat()
            parents = [parent.lstat() for parent in resolved.parents]
        except OSError:
            continue
        nodes = [leaf, *parents]
        system_uid = leaf.st_uid
        writable = leaf.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
        if (
            not resolved.is_absolute()
            or not stat.S_ISDIR(leaf.st_mode)
            or not _consistent_system_owner(nodes, effective_uid)
            or (writable and not leaf.st_mode & stat.S_ISVTX)
        ):
            continue
        if any(
            not stat.S_ISDIR(item.st_mode)
            or item.st_uid != system_uid
            or (item.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
                and not item.st_mode & stat.S_ISVTX)
            for item in nodes[1:]
        ):
            continue
        if any(_is_within(resolved, protected) for protected in protected_paths):
            continue
        return resolved
    raise VerificationError("trusted system temporary directory unavailable")


class Provider:
    """Noninteractive GitHub and SSH Git measurement boundary."""

    def __init__(self) -> None:
        self._gh_ready = False
        self._git_path = _trusted_executable(GIT_CANDIDATES, "git")
        self._gh_path: Path | None = None
        self._ssh_path = _trusted_executable(SSH_CANDIDATES, "ssh")
        self._false_path = _trusted_executable(FALSE_CANDIDATES, "false")
        caller_paths = [Path.cwd().resolve(strict=True)]
        for key in (
            "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME",
            "XDG_DATA_HOME", "GH_CONFIG_DIR",
        ):
            caller_path = _caller_path(os.environ.get(key))
            if caller_path is not None:
                caller_paths.append(caller_path)
        self._protected_paths = tuple(dict.fromkeys(caller_paths))
        self._temp_base = _trusted_temp_base(self._protected_paths)

    @staticmethod
    def _base_env() -> dict[str, str]:
        return {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}

    def _git_environment(self) -> dict[str, str]:
        environment = self._base_env()
        environment.update({
            "GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1",
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_ATTR_NOSYSTEM": "1",
            "GIT_TERMINAL_PROMPT": "0", "GIT_ASKPASS": str(self._false_path),
            "SSH_ASKPASS": str(self._false_path),
            "GIT_SSH_COMMAND": (
                f"{self._ssh_path} -F /dev/null -o HostName=github.com "
                "-o HostKeyAlias=github.com -o CanonicalizeHostname=no "
                "-o ProxyCommand=none -o ProxyJump=none "
                "-o StrictHostKeyChecking=yes -o PermitLocalCommand=no "
                "-o BatchMode=yes -o PasswordAuthentication=no "
                "-o KbdInteractiveAuthentication=no"
            ),
        })
        return environment

    def _git(self, root: Path, *args: str, raw: bool = False) -> str:
        env = self._git_environment()
        protected = [
            str(self._git_path), "--no-optional-locks", "-c", "core.fsmonitor=false",
            "-c", "core.hooksPath=/dev/null", "-c", "diff.external=",
        ]
        if args and args[0] == "diff":
            command = [*protected, "diff", "--no-ext-diff", "--no-textconv", *args[1:]]
        else:
            command = [*protected, *args]
        return _completed(command, cwd=root, env=env, strip=not raw)

    def _network_git(self, *args: str) -> str:
        env = self._git_environment()
        # Run outside every checkout so repository-controlled config and URL
        # rewrites cannot affect the explicit canonical SSH endpoint.
        return _completed([str(self._git_path), *args], cwd=Path("/"), env=env)

    def _gh(self, *args: str) -> dict[str, Any]:
        token = os.environ.get("GITHUB_TOKEN")
        if not token:
            raise VerificationError("GITHUB_TOKEN is required; credential stores are forbidden")
        if self._gh_path is None:
            self._gh_path = _trusted_executable(GH_CANDIDATES, "gh")
        gh_path = self._gh_path
        with tempfile.TemporaryDirectory(
            prefix="telegram-report-gh-", dir=self._temp_base
        ) as sandbox_value:
            sandbox = Path(sandbox_value).resolve(strict=True)
            if any(_is_within(sandbox, protected) for protected in self._protected_paths):
                raise VerificationError("gh sandbox overlaps caller-controlled path")
            if sandbox.parent != self._temp_base or sandbox.stat().st_mode & 0o777 != 0o700:
                raise VerificationError("gh sandbox is not an isolated mode-0700 directory")
            working_directory = sandbox / "work"
            isolated_directories = {
                "HOME": sandbox / "home",
                "XDG_CONFIG_HOME": sandbox / "config",
                "XDG_STATE_HOME": sandbox / "state",
                "XDG_CACHE_HOME": sandbox / "cache",
                "GH_CONFIG_DIR": sandbox / "gh-config",
            }
            for directory in (*isolated_directories.values(), working_directory):
                directory.mkdir(mode=0o700)
            env = self._base_env()
            env.update({
                "GH_TOKEN": token, "GH_PROMPT_DISABLED": "1", "GH_NO_UPDATE_NOTIFIER": "1",
                "GH_PAGER": "cat", "PAGER": "cat",
                **{name: str(path) for name, path in isolated_directories.items()},
            })
            if not self._gh_ready:
                version = _completed(
                    [str(gh_path), "--version"], cwd=working_directory, env=env
                ).splitlines()[0]
                match = re.fullmatch(r"gh version ([0-9]+)\.([0-9]+)\.([0-9]+).*", version)
                if match is None or tuple(map(int, match.groups())) < GH_VERSION:
                    raise VerificationError("gh >= 2.97.0 is required")
                self._gh_ready = True
            raw = _completed([str(gh_path), *args], cwd=working_directory, env=env)
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise VerificationError("gh returned invalid JSON") from exc
        if not isinstance(value, dict):
            raise VerificationError("gh returned non-object JSON")
        return value

    def repository(self, root: Path, repository_id: str) -> None:
        absolute = Path(os.path.abspath(root))
        resolved = root.resolve(strict=True)
        if absolute != resolved:
            raise VerificationError("repository root is not canonical")
        expected = f"git@github.com:{repository_id}.git"
        fetch = self._git(root, "remote", "get-url", "--all", "origin").splitlines()
        push = self._git(root, "remote", "get-url", "--push", "--all", "origin").splitlines()
        if fetch != [expected] or push != [expected]:
            raise VerificationError("repository origin is not the canonical SSH remote")
        config = self._git(root, "config", "--local", "--list").lower().splitlines()
        forbidden = ("include.", "includeif.", "url.", "core.fsmonitor",
                     "core.hookspath", "remote.origin.uploadpack",
                     "extensions.partialclone")
        if any(line.split("=", 1)[0].startswith(forbidden) for line in config):
            raise VerificationError("repository local config contains active execution or URL rewrite")
        config_keys = [line.split("=", 1)[0] for line in config]
        if any(
            key.startswith("remote.")
            and (key.endswith(".promisor") or key.endswith(".partialclonefilter"))
            for key in config_keys
        ):
            raise VerificationError("repository local config contains promisor state")
        ssh_commands = [line.split("=", 1)[1] for line in config
                        if line.startswith("core.sshcommand=")]
        allowed_ssh = "ssh -o batchmode=yes -o kbdinteractiveauthentication=no -o passwordauthentication=no"
        if ssh_commands not in ([], [allowed_ssh]):
            raise VerificationError("repository local SSH command is not noninteractive")
        top = Path(self._git(root, "rev-parse", "--show-toplevel")).resolve(strict=True)
        if top != resolved:
            raise VerificationError("repository root mismatch")
        if self._git(root, "rev-parse", "--is-shallow-repository") != "false":
            raise VerificationError("shallow repository is forbidden")
        grafts = Path(self._git(root, "rev-parse", "--git-path", "info/grafts"))
        if not grafts.is_absolute():
            grafts = root / grafts
        if os.path.lexists(grafts):
            raise VerificationError("repository grafts are forbidden")
        if self._git(root, "for-each-ref", "--format=%(refname)", "refs/replace"):
            raise VerificationError("repository replacement refs are forbidden")

    def remote_head(self, root: Path, repository_id: str, branch: str) -> str:
        remote = f"git@github.com:{repository_id}.git"
        output = self._network_git("ls-remote", "--heads", remote, f"refs/heads/{branch}")
        lines = output.splitlines()
        if len(lines) != 1:
            raise VerificationError(f"remote branch unavailable or ambiguous: {branch}")
        fields = lines[0].split("\t")
        if len(fields) != 2 or fields[1] != f"refs/heads/{branch}" or not renderer.SHA_RE.fullmatch(fields[0]):
            raise VerificationError(f"invalid remote branch response: {branch}")
        return fields[0]

    def target_contains(self, root: Path, repository_id: str, branch: str,
                        merge_sha: str) -> str:
        target_head = self.remote_head(root, repository_id, branch)
        fetched_target = self._git(root, "rev-parse", f"refs/remotes/origin/{branch}")
        if fetched_target != target_head:
            raise VerificationError(
                "fetched target ref does not equal measured SSH head; prefetch exact ref first"
            )
        self._git(root, "cat-file", "-e", f"{merge_sha}^{{commit}}")
        try:
            self._git(root, "merge-base", "--is-ancestor", merge_sha, fetched_target)
        except VerificationError as exc:
            raise VerificationError("merge SHA is not contained in SSH target") from exc
        return target_head

    def changed_paths(self, root: Path, base_sha: str, candidate_sha: str) -> list[str]:
        """Measure the exact PR base-to-candidate path set from trusted Git objects."""
        base_sha = renderer.sha(base_sha, "pr.baseRefOid")
        candidate_sha = renderer.sha(candidate_sha, "candidate_sha")
        self._git(root, "cat-file", "-e", f"{base_sha}^{{commit}}")
        self._git(root, "cat-file", "-e", f"{candidate_sha}^{{commit}}")
        raw = self._git(
            root, "diff", "--name-only", "--diff-filter=ACDMRTUXB", "-z",
            base_sha, candidate_sha, "--", raw=True,
        )
        if not raw or not raw.endswith("\0"):
            raise VerificationError("changed files are empty or malformed")
        values = raw[:-1].split("\0")
        try:
            return renderer.changed_files(values)
        except renderer.ReportError as exc:
            raise VerificationError(str(exc)) from exc

    def pull_request(self, repository_id: str, number: int) -> dict[str, Any]:
        value = self._gh(
            "pr", "view", str(number), "--repo", repository_id, "--json",
            "number,url,headRefOid,headRefName,baseRefOid,baseRefName,state,mergeCommit,files",
        )
        expected_url = f"https://github.com/{repository_id}/pull/{number}"
        if value.get("number") != number or value.get("url") != expected_url:
            raise VerificationError("GitHub PR repository/number mismatch")
        return value

    def run(self, repository_id: str, run_id: int) -> dict[str, Any]:
        value = self._gh(
            "run", "view", str(run_id), "--repo", repository_id, "--json",
            "databaseId,url,headSha,headBranch,status,conclusion,event,attempt",
        )
        expected_url = f"https://github.com/{repository_id}/actions/runs/{run_id}"
        if value.get("databaseId") != run_id or value.get("url") != expected_url:
            raise VerificationError("GitHub run repository/id mismatch")
        attempt = value.get("attempt")
        if isinstance(attempt, bool) or not isinstance(attempt, int) or attempt < 1:
            raise VerificationError("GitHub run attempt is invalid")
        jobs_value = self._gh(
            "run", "view", str(run_id), "--repo", repository_id,
            "--attempt", str(attempt), "--json", "databaseId,url,attempt,jobs",
        )
        expected_attempt_url = f"{expected_url}/attempts/{attempt}"
        if (
            jobs_value.get("databaseId") != run_id
            or jobs_value.get("attempt") != attempt
            or jobs_value.get("url") != expected_attempt_url
        ):
            raise VerificationError("GitHub run attempt repository/id mismatch")
        jobs = jobs_value.get("jobs")
        if not isinstance(jobs, list):
            raise VerificationError("GitHub run jobs unavailable")
        value["jobs"] = jobs
        return value


def _number(value: str, pattern: str, where: str) -> int:
    match = re.fullmatch(pattern, value)
    if match is None:
        raise VerificationError(f"{where}: invalid provider URL")
    return int(match.group(1))


SECRET_PATTERNS = tuple(map(re.compile, (
    r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
    r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
    r"\bAKIA[0-9A-Z]{16}\b",
    r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b",
    r"\b[0-9]{8,10}:[A-Za-z0-9_-]{35}\b",
    r"-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----",
)))
OPENAI_SECRET_CANDIDATE = re.compile(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{32,}\b")


def _looks_like_openai_secret(candidate: str) -> bool:
    body = (candidate.removeprefix("sk-proj-") if candidate.startswith("sk-proj-")
            else candidate.removeprefix("sk-"))
    minimum = 40 if candidate.startswith("sk-proj-") else 32
    return (
        len(body) >= minimum
        and any(character.islower() for character in body)
        and any(character.isupper() for character in body)
        and any(character.isdigit() for character in body)
        and len(set(body)) >= 12
    )


def _strings(value: Any) -> Iterator[str]:
    stack = [value]
    seen: set[int] = set()
    while stack:
        current = stack.pop()
        if isinstance(current, str):
            yield current
        elif isinstance(current, dict):
            marker = id(current)
            if marker in seen:
                continue
            seen.add(marker)
            for key, item in current.items():
                stack.extend((key, item))
        elif isinstance(current, (list, tuple)):
            marker = id(current)
            if marker in seen:
                continue
            seen.add(marker)
            stack.extend(current)


def _reject_secrets(value: Any) -> None:
    environment_secrets = tuple(
        secret for name in ("GITHUB_TOKEN", "GH_TOKEN")
        if (secret := os.environ.get(name))
    )
    for candidate in _strings(value):
        if any(secret in candidate for secret in environment_secrets):
            raise VerificationError("report contains process credential material")
        if any(pattern.search(candidate) for pattern in SECRET_PATTERNS):
            raise VerificationError("report contains credential-like material")
        if any(_looks_like_openai_secret(match.group(0))
               for match in OPENAI_SECRET_CANDIDATE.finditer(candidate)):
            raise VerificationError("report contains credential-like material")


def _verify_run(provider: Provider, repository_id: str, claimed: dict[str, Any],
                expected_sha: str, expected_branch: str | None = None
                ) -> tuple[dict[str, Any], list[str], dict[str, Any]]:
    claimed = renderer.strict_keys(claimed, {"state", "head_sha", "url"}, "ci")
    run_id = _number(
        renderer.one_line(claimed["url"], "ci.url"),
        rf"https://github\.com/{re.escape(repository_id)}/actions/runs/([1-9][0-9]*)", "ci.url",
    )
    run = provider.run(repository_id, run_id)
    if run.get("status") != "completed" or run.get("conclusion") != "success":
        raise VerificationError("GitHub run is not completed successfully")
    actual_state = "success"
    if run.get("headSha") != expected_sha or claimed.get("head_sha") != expected_sha:
        raise VerificationError("GitHub run head SHA mismatch")
    if claimed.get("state") != actual_state:
        raise VerificationError("GitHub run state mismatch")
    if expected_branch is not None and run.get("headBranch") != expected_branch:
        raise VerificationError("GitHub run branch mismatch")
    normalized = {"state": actual_state, "head_sha": run["headSha"], "url": run["url"]}
    jobs = run.get("jobs")
    if not isinstance(jobs, list):
        raise VerificationError("GitHub run jobs unavailable")
    normalized_jobs = []
    for job in jobs:
        if not isinstance(job, dict) or not isinstance(job.get("name"), str):
            raise VerificationError("GitHub run job invalid")
        outcome = job.get("conclusion") or job.get("status")
        if not isinstance(outcome, str):
            raise VerificationError("GitHub run job outcome invalid")
        normalized_jobs.append(f"{job['name']}: {outcome}")
    return normalized, normalized_jobs, run


def _bind_reported_jobs(data: dict[str, Any], normalized_jobs: list[str]) -> None:
    claimed_jobs = sorted(renderer.string_list(data["tests"], "tests"))
    actual_jobs = sorted(normalized_jobs)
    if claimed_jobs != actual_jobs:
        raise VerificationError("reported tests do not equal GitHub run jobs")
    data["tests"] = actual_jobs


def _verify_pr(provider: Provider, repository_id: str, pr_value: str, expected_sha: str,
               head_branch: str, base_branch: str, base_sha: str | None,
               states: set[str]) -> dict[str, Any]:
    number = _number(
        pr_value, rf"https://github\.com/{re.escape(repository_id)}/pull/([1-9][0-9]*)", "pr_url"
    )
    pr = provider.pull_request(repository_id, number)
    if (pr.get("headRefOid") != expected_sha or pr.get("headRefName") != head_branch
            or pr.get("baseRefName") != base_branch or pr.get("state") not in states):
        raise VerificationError("GitHub PR head/base/state mismatch")
    if base_sha is not None and pr.get("baseRefOid") != base_sha:
        raise VerificationError("GitHub PR base SHA mismatch")
    return pr


def _verify_postmerge(data: dict[str, Any], root: Path, provider: Provider,
                      checks: list[str]) -> None:
    renderer.strict_keys(data, RAW_POSTMERGE_KEYS, "report")
    repository_id = renderer.repository(data["repository"])
    provider.repository(root, repository_id)
    source = renderer.branch(data["source_branch"], "source_branch")
    target = renderer.branch(data["target_branch"], "target_branch")
    candidate = renderer.sha(data["candidate_sha"], "candidate_sha")
    merge_sha = renderer.sha(data["merge_sha"], "merge_sha")
    target_head = provider.target_contains(root, repository_id, target, merge_sha)
    pr = _verify_pr(provider, repository_id, data["pr_url"], candidate,
                    source, target, None, {"MERGED"})
    merge_commit = pr.get("mergeCommit")
    if not isinstance(merge_commit, dict) or merge_commit.get("oid") != merge_sha:
        raise VerificationError("GitHub PR merge SHA mismatch")
    base_sha = renderer.sha(pr.get("baseRefOid"), "pr.baseRefOid")
    provider_files = pr.get("files")
    if not isinstance(provider_files, list):
        raise VerificationError("GitHub PR changed files unavailable")
    try:
        provider_paths = renderer.changed_files([
            item.get("path") if isinstance(item, dict) else None
            for item in provider_files
        ])
    except renderer.ReportError as exc:
        raise VerificationError(f"GitHub PR changed files invalid: {exc}") from exc
    git_paths = provider.changed_paths(root, base_sha, candidate)
    if sorted(provider_paths) != sorted(git_paths):
        raise VerificationError("GitHub PR and Git changed files mismatch")
    data["files"] = sorted(git_paths)
    normalized_ci, normalized_jobs, run = _verify_run(
        provider, repository_id, data["ci"], merge_sha, target
    )
    run_id = int(normalized_ci["url"].rsplit("/", 1)[-1])
    if (run.get("event") != "push" or run.get("status") != "completed"
            or run.get("conclusion") != "success"):
        raise VerificationError(
            "postmerge requires completed successful target-branch push workflow"
        )
    data["pr_url"] = pr["url"]
    data["ci"] = {"state": "success", "head_sha": merge_sha, "url": run["url"]}
    _bind_reported_jobs(data, normalized_jobs)
    checks.extend([f"ssh-target:{target}@{target_head}:contains:{merge_sha}",
                   f"github-pr:{pr['number']}", f"github-run:{run_id}"])


def _verify_correction(data: dict[str, Any], root: Path, provider: Provider,
                       checks: list[str]) -> None:
    renderer.strict_keys(data, renderer.EVENT_KEYS["correction"], "report")
    message_id = data.get("prior_message_id")
    if not isinstance(message_id, int) or isinstance(message_id, bool) or message_id < 1:
        raise VerificationError("correction requires a positive merge-report receipt")
    repository_id = renderer.repository(data["repository"])
    provider.repository(root, repository_id)
    context_sha = renderer.sha(data["context_sha"], "context_sha")
    field_type = renderer.one_line(data.get("field_type"), "field_type")
    field = renderer.one_line(data.get("field"), "field")
    if field_type not in renderer.CORRECTION_FIELDS or field not in renderer.CORRECTION_FIELDS[field_type]:
        raise VerificationError("correction field/type is not a merge-report field")
    evidence = renderer.strict_keys(data["evidence"], {"source", "url", "head_sha", "ref"}, "evidence")
    if renderer.sha(evidence["head_sha"], "evidence.head_sha") != context_sha:
        raise VerificationError("correction evidence head mismatch")
    source = renderer.one_line(evidence["source"], "evidence.source")
    ref = renderer.branch(evidence["ref"], "evidence.ref")
    if source == "github_commit":
        if field != "commit_url":
            raise VerificationError("commit evidence is valid only for merge commit URL")
        renderer.github_commit_url(evidence["url"], repository_id, context_sha, "evidence.url")
        target_head = provider.target_contains(root, repository_id, ref, context_sha)
        if data.get("replacement") != evidence["url"]:
            raise VerificationError("merge commit URL correction differs from evidence")
        checks.append(f"ssh-target:{ref}@{target_head}:contains:{context_sha}")
    elif source == "github_pull_request":
        number = _number(evidence["url"], rf"https://github\.com/{re.escape(repository_id)}/pull/([1-9][0-9]*)", "evidence.url")
        pr = provider.pull_request(repository_id, number)
        merge_commit = pr.get("mergeCommit")
        if (pr.get("state") != "MERGED" or not isinstance(merge_commit, dict)
                or merge_commit.get("oid") != context_sha):
            raise VerificationError("correction PR is not the exact merged PR")
        if field == "source_branch":
            expected_ref = pr.get("headRefName")
        elif field == "target_branch":
            expected_ref = pr.get("baseRefName")
        elif field == "pr_url":
            expected_ref = pr.get("baseRefName")
            if data.get("replacement") != pr.get("url"):
                raise VerificationError("merged PR URL correction differs from provider")
        else:
            raise VerificationError("PR evidence is valid only for merge branch or PR URL")
        if ref != expected_ref or (field_type == "branch" and data.get("replacement") != expected_ref):
            raise VerificationError("correction merged PR branch/ref mismatch")
        target_head = provider.target_contains(
            root, repository_id, renderer.branch(pr.get("baseRefName"), "pr.baseRefName"), context_sha
        )
        data["evidence"]["url"] = pr["url"]
        checks.extend([f"github-pr:{number}",
                       f"ssh-target:{pr['baseRefName']}@{target_head}:contains:{context_sha}"])
    elif source == "github_actions_run":
        if field not in {"ci.state", "ci.url"}:
            raise VerificationError("run evidence is valid only for merge CI fields")
        run_id = _number(evidence["url"], rf"https://github\.com/{re.escape(repository_id)}/actions/runs/([1-9][0-9]*)", "evidence.url")
        run = provider.run(repository_id, run_id)
        if run.get("headSha") != context_sha or run.get("headBranch") != ref:
            raise VerificationError("correction run head/ref mismatch")
        if (run.get("event") != "push" or run.get("status") != "completed"
                or run.get("conclusion") != "success"):
            raise VerificationError("correction run is not successful merge-SHA push CI")
        if field == "ci.state" and data.get("replacement") != "success":
            raise VerificationError("correction CI state differs from provider")
        if field == "ci.url" and data.get("replacement") != run.get("url"):
            raise VerificationError("correction CI URL differs from provider")
        target_head = provider.target_contains(root, repository_id, ref, context_sha)
        data["evidence"]["url"] = run["url"]
        checks.extend([f"github-run:{run_id}",
                       f"ssh-target:{ref}@{target_head}:contains:{context_sha}"])
    else:
        raise VerificationError("correction evidence source unsupported")


def verify_and_render(payload: Any, root: Path, provider: Provider | None = None) -> str:
    if not isinstance(payload, dict):
        raise VerificationError("report: expected object")
    data = copy.deepcopy(payload)
    _reject_secrets(data)
    resolved_root = root.resolve(strict=True)
    if Path(os.path.abspath(root)) != resolved_root or resolved_root != SKILL_REPOSITORY_ROOT:
        raise VerificationError("verifier must run from its canonical repository root")
    event = data.get("event")
    provider = provider or Provider()
    checks: list[str] = []
    if event == "postmerge":
        _verify_postmerge(data, root, provider, checks)
    elif event == "correction":
        _verify_correction(data, root, provider, checks)
    else:
        raise VerificationError("event: unsupported")
    rendered = renderer.render(renderer._verified_report(data, checks))
    _reject_secrets(rendered)
    return rendered


def _read_payload(stream: BinaryIO) -> Any:
    raw = stream.read(INPUT_LIMIT + 1)
    if len(raw) > INPUT_LIMIT:
        raise VerificationError("report input exceeds size limit")
    value = json.loads(raw.decode("utf-8"))
    stack = [(value, 0)]
    while stack:
        current, depth = stack.pop()
        if depth > MAX_NESTING_DEPTH:
            raise VerificationError("report input exceeds nesting limit")
        if isinstance(current, dict):
            stack.extend((item, depth + 1) for item in current.values())
        elif isinstance(current, list):
            stack.extend((item, depth + 1) for item in current)
    return value


def main() -> int:
    try:
        payload = _read_payload(sys.stdin.buffer)
        rendered = verify_and_render(payload, Path.cwd())
    except (KeyError, OSError, RecursionError, TypeError, UnicodeError, json.JSONDecodeError,
            renderer.ReportError, VerificationError) as exc:
        print(json.dumps({"valid": False, "errors": [str(exc)]},
                         ensure_ascii=False, sort_keys=True), file=sys.stderr)
        return 1
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
