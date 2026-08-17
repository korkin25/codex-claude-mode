#!/usr/bin/env python3
"""Run only the externally authenticated inspector blob from an admitted base."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
INSPECTOR_PATH = ".agents/skills/ccm-delivery-executor/scripts/inspect_state.py"
GIT_CANDIDATES = (Path("/usr/bin/git"), Path("/bin/git"))
TIMEOUT_SECONDS = 5.0
MAX_INSPECTOR_BYTES = 4 * 1024 * 1024
MAX_ROOT_CONFIG_BYTES = 4096
ROOT_CONFIG_SUFFIX = (".config", "codex-claude-mode", "delivery-executor.json")


class PreflightError(RuntimeError):
    pass


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
                   and (index == 0 or stat.S_ISDIR(item.st_mode))
                   for index, item in enumerate(metadata)):
            return resolved
    raise PreflightError("TRUSTED_GIT_UNAVAILABLE")


def git_environment() -> dict[str, str]:
    return {
        "GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1",
        "GIT_TERMINAL_PROMPT": "0", "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull, "GIT_ATTR_NOSYSTEM": "1",
        "GIT_OPTIONAL_LOCKS": "0", "HOME": os.devnull,
        "XDG_CONFIG_HOME": os.devnull, "LC_ALL": "C",
    }


def terminate(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except OSError:
            process.kill()
    process.wait()


def base_inspector(root: Path, base_sha: str) -> bytes:
    command = [
        str(trusted_git()), "--no-pager", "--no-optional-locks",
        "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
        "-c", "diff.external=", "-c", "core.attributesFile=/dev/null",
        "-c", "protocol.ext.allow=never", "-c", "protocol.file.allow=never",
        "show", f"{base_sha}:{INSPECTOR_PATH}",
    ]
    try:
        process = subprocess.Popen(
            command, cwd=root, env=git_environment(), stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
        )
    except OSError as exc:
        raise PreflightError("BASE_INSPECTOR_GIT_UNAVAILABLE") from exc
    assert process.stdout is not None and process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    deadline = time.monotonic() + TIMEOUT_SECONDS
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                terminate(process)
                raise PreflightError("BASE_INSPECTOR_GIT_TIMEOUT")
            events = selector.select(remaining)
            if not events:
                terminate(process)
                raise PreflightError("BASE_INSPECTOR_GIT_TIMEOUT")
            for key, _ in events:
                stream = key.fileobj
                chunk = os.read(stream.fileno(), 65_536)
                if not chunk:
                    selector.unregister(stream)
                    stream.close()
                    continue
                buffer = buffers[key.data]
                buffer.extend(chunk)
                if len(buffer) > MAX_INSPECTOR_BYTES:
                    terminate(process)
                    raise PreflightError(f"BASE_INSPECTOR_{key.data.upper()}_LIMIT")
        returncode = process.wait()
    finally:
        selector.close()
        for stream in (process.stdout, process.stderr):
            if not stream.closed:
                stream.close()
    if returncode != 0:
        raise PreflightError("BASE_INSPECTOR_UNAVAILABLE")
    return bytes(buffers["stdout"])


def option_value(arguments: list[str], option: str) -> str | None:
    positions = [index for index, value in enumerate(arguments) if value == option]
    if len(positions) != 1 or positions[0] + 1 >= len(arguments):
        return None
    return arguments[positions[0] + 1]


def reserved_option(argument: str, option: str) -> bool:
    """Reject exact, attached, and argparse-prefix spellings of an injected option."""
    name = argument.split("=", 1)[0]
    return name.startswith("--") and (name == option or option.startswith(name))


def owner_root_config() -> Path:
    """Return the fixed per-owner configuration path without trusting HOME/XDG variables."""
    try:
        owner_home = Path(pwd.getpwuid(os.geteuid()).pw_dir)
    except KeyError as exc:
        raise PreflightError("PREFLIGHT_OWNER_HOME_UNAVAILABLE") from exc
    return owner_home.joinpath(*ROOT_CONFIG_SUFFIX)


def unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    document: dict[str, object] = {}
    for key, value in pairs:
        if key in document:
            raise PreflightError(f"PREFLIGHT_ROOT_CONFIG_DUPLICATE_KEY {key}")
        document[key] = value
    return document


def configured_task_root() -> Path:
    """Load the private owner-selected root from a fixed, non-symlink configuration file."""
    config = owner_root_config()
    try:
        parent = config.parent
        if parent.resolve(strict=True) != parent or config.resolve(strict=True) != config:
            raise PreflightError("PREFLIGHT_ROOT_CONFIG_NONCANONICAL")
        metadata = config.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid():
            raise PreflightError("PREFLIGHT_ROOT_CONFIG_UNTRUSTED")
        if stat.S_IMODE(metadata.st_mode) != 0o600:
            raise PreflightError("PREFLIGHT_ROOT_CONFIG_PERMISSIONS")
        descriptor = os.open(config, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        try:
            opened = os.fstat(descriptor)
            if (opened.st_dev, opened.st_ino) != (metadata.st_dev, metadata.st_ino):
                raise PreflightError("PREFLIGHT_ROOT_CONFIG_CHANGED")
            raw = os.read(descriptor, MAX_ROOT_CONFIG_BYTES + 1)
        finally:
            os.close(descriptor)
    except PreflightError:
        raise
    except OSError as exc:
        raise PreflightError("PREFLIGHT_ROOT_CONFIG_UNAVAILABLE") from exc
    if len(raw) > MAX_ROOT_CONFIG_BYTES:
        raise PreflightError("PREFLIGHT_ROOT_CONFIG_LIMIT")
    try:
        document = json.loads(raw, object_pairs_hook=unique_json_object)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise PreflightError("PREFLIGHT_ROOT_CONFIG_INVALID") from exc
    if (not isinstance(document, dict) or set(document) != {"schema_version", "canonical_root"}
            or type(document.get("schema_version")) is not int
            or document.get("schema_version") != 1
            or not isinstance(document.get("canonical_root"), str)):
        raise PreflightError("PREFLIGHT_ROOT_CONFIG_INVALID")
    return canonical_root(Path(document["canonical_root"]))


def canonical_root(root: Path) -> Path:
    lexical = Path(os.path.abspath(root))
    try:
        leaf = lexical.lstat()
        resolved = lexical.resolve(strict=True)
    except OSError as exc:
        raise PreflightError("PREFLIGHT_ROOT_UNAVAILABLE") from exc
    if (root != lexical or stat.S_ISLNK(leaf.st_mode) or not stat.S_ISDIR(leaf.st_mode)
            or resolved != lexical):
        raise PreflightError("PREFLIGHT_ROOT_NONCANONICAL")
    return lexical


def run_authenticated_inspector(root: Path, base_sha: str, inspector_digest: str,
                                arguments: list[str]) -> int:
    if not SHA_RE.fullmatch(base_sha) or not DIGEST_RE.fullmatch(inspector_digest):
        raise PreflightError("PREFLIGHT_INPUT_FORMAT")
    root = canonical_root(root)
    if not arguments or arguments[0] != "inspect":
        raise PreflightError("PREFLIGHT_INSPECT_COMMAND_REQUIRED")
    if any(reserved_option(argument, "--root") for argument in arguments[1:]):
        raise PreflightError("PREFLIGHT_RESERVED_ROOT_ARGUMENT")
    if option_value(arguments, "--public-main-head") != base_sha:
        raise PreflightError("PREFLIGHT_BASE_BINDING_MISMATCH")
    if option_value(arguments, "--inspector-digest") != inspector_digest:
        raise PreflightError("PREFLIGHT_DIGEST_BINDING_MISMATCH")
    raw = base_inspector(root, base_sha)
    measured = "sha256:" + hashlib.sha256(raw).hexdigest()
    if measured != inspector_digest:
        raise PreflightError("PREFLIGHT_BASE_INSPECTOR_DIGEST_MISMATCH")
    with tempfile.TemporaryDirectory(prefix="ccm-inspector-") as directory:
        extracted = Path(directory) / "inspect_state.py"
        descriptor = os.open(extracted, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(raw)
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(extracted, 0o400)
            read_descriptor = os.open(extracted, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
            try:
                stable_path = f"/dev/fd/{read_descriptor}"
                completed = subprocess.run(
                    [sys.executable, "-I", stable_path, arguments[0], "--root", str(root),
                     *arguments[1:]],
                    cwd=root, env={"LC_ALL": "C", "PYTHONDONTWRITEBYTECODE": "1"},
                    stdin=subprocess.DEVNULL, pass_fds=(read_descriptor,), check=False,
                )
            finally:
                os.close(read_descriptor)
        finally:
            if not extracted.exists():
                raise PreflightError("PREFLIGHT_IMMUTABLE_INSPECTOR_LOST")
    return completed.returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--task-root", required=True, type=Path)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--inspector-digest", required=True)
    parser.add_argument("inspector_arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    arguments = args.inspector_arguments
    if arguments[:1] == ["--"]:
        arguments = arguments[1:]
    try:
        configured_root = configured_task_root()
        supplied_root = canonical_root(Path(os.path.abspath(args.task_root)))
        if supplied_root != configured_root or args.task_root != configured_root:
            raise PreflightError("PREFLIGHT_ROOT_BINDING_MISMATCH")
        return run_authenticated_inspector(
            configured_root, args.base_sha,
            args.inspector_digest, arguments,
        )
    except (OSError, PreflightError) as exc:
        print(json.dumps({"admitted": False, "classification": "invalid", "errors": [str(exc)]},
                         sort_keys=True))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
