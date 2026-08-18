#!/usr/bin/env python3
"""Detect that the upstream source of a vendored skill has moved ahead.

`tests/test_telegram_delivery_reporter.py` pins the *local* copy of
`.agents/skills/telegram-delivery-reporter/` to a recorded digest. That gate is
offline and it only proves the copy has not been edited in place; it cannot see
the other failure mode — the canonical implementation in the private controller
repository gaining commits while this copy stays frozen at an older revision.

This script closes that gap. Git tree objects are content-addressed and carry
only names relative to themselves, so the tree OID of the vendored directory is
identical to the tree OID of the source directory whenever the two hold the same
bytes and modes. One `gh api` call therefore settles the question: ask GitHub
for the source directory's current tree OID on the tracked ref and compare it
with the recorded one.

The call reads a private repository, so it is deliberately kept out of the
offline unittest set and out of the per-push CI gates: pull requests from forks
receive no secrets, and a check that cannot authenticate must not be able to
turn the public repository's CI red. When the source cannot be read the script
says so and succeeds, unless `--require-source-access` is given.

Exit codes:
    0  the source tree matches the recorded OID, or the source is unreadable
       and `--require-source-access` was not given
    1  the source moved ahead of the vendored copy; re-synchronisation is due
    2  the source is unreadable and `--require-source-access` was given
    3  usage error
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from typing import Any, Callable


# The vendored copy and its recorded upstream revision. `SOURCE_TREE_OID` is the
# Git tree object of `SOURCE_PATH` at `SOURCE_SHA`, obtained with
# `git -C <ccm-multi> rev-parse <SOURCE_SHA>:<SOURCE_PATH>`. Because a tree OID
# depends only on the directory's own contents, the vendored copy in this
# repository hashes to the very same OID; `tests/test_vendored_skill_source.py`
# asserts that identity offline, so this constant can never drift away from the
# bytes that are actually checked in.
SOURCE_REPOSITORY = "korkin25/ccm-multi"
SOURCE_REF = "main"
SOURCE_PATH = ".agents/skills/telegram-delivery-reporter"
SOURCE_SHA = "47cf085e3d82e8cdb57af7bbc01e21a95ae3d861"
SOURCE_TREE_OID = "2b1e01a99655fb103db1071367dd269a2e72ae3f"

TREE_OID_RE = re.compile(r"^[0-9a-f]{40}$")
GH_TIMEOUT_SECONDS = 60

EXIT_OK = 0
EXIT_DRIFT = 1
EXIT_UNAVAILABLE = 2
EXIT_USAGE = 3

STATUS_SYNCHRONISED = "synchronised"
STATUS_DRIFTED = "drifted"
STATUS_UNAVAILABLE = "unavailable"


class Probe:
    """Outcome of one attempt to read the source directory's tree OID."""

    def __init__(self, status: str, detail: str, tree_oid: str | None = None) -> None:
        self.status = status
        self.detail = detail
        self.tree_oid = tree_oid


def parent_and_leaf(path: str) -> tuple[str, str]:
    parent, _, leaf = path.rpartition("/")
    return parent, leaf


def gh_argv() -> list[str]:
    """The single read-only API call this check is allowed to make.

    The *parent* directory is listed rather than the skill directory itself.
    GitHub answers a missing path and an inaccessible private repository with
    the same 404, so asking for the skill directory directly would make a
    deleted or relocated source indistinguishable from a missing token. Listing
    the parent separates the two: a 404 means the repository or the parent path
    is unreadable, while a successful listing that no longer carries the skill
    entry is real drift.
    """
    parent, _ = parent_and_leaf(SOURCE_PATH)
    return [
        "gh", "api",
        "-H", "Accept: application/vnd.github+json",
        "-H", "X-GitHub-Api-Version: 2022-11-28",
        f"repos/{SOURCE_REPOSITORY}/contents/{parent}?ref={SOURCE_REF}",
    ]


def run_gh(argv: list[str]) -> subprocess.CompletedProcess[str]:
    """Run `gh` non-interactively without touching a credential store."""
    environment = dict(os.environ)
    environment["GH_PROMPT_DISABLED"] = "1"
    environment["GH_NO_UPDATE_NOTIFIER"] = "1"
    environment["GH_PAGER"] = "cat"
    environment["CLICOLOR"] = "0"
    return subprocess.run(
        argv, env=environment, text=True, check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=GH_TIMEOUT_SECONDS,
    )


def summarise_failure(completed: subprocess.CompletedProcess[str]) -> str:
    """Describe a failed `gh` call without echoing anything token-shaped."""
    text = (completed.stderr or completed.stdout or "").strip()
    first = next((line.strip() for line in text.splitlines() if line.strip()), "")
    first = re.sub(r"gh[pousr]_[A-Za-z0-9]{10,}", "<redacted>", first)
    if len(first) > 200:
        first = first[:197] + "..."
    return first or f"gh exited with status {completed.returncode}"


def probe_source(runner: Callable[[list[str]], subprocess.CompletedProcess[str]]) -> Probe:
    """Ask GitHub for the current tree OID of the source skill directory."""
    parent, leaf = parent_and_leaf(SOURCE_PATH)
    try:
        completed = runner(gh_argv())
    except FileNotFoundError:
        return Probe(STATUS_UNAVAILABLE, "the `gh` CLI is not installed")
    except subprocess.TimeoutExpired:
        return Probe(
            STATUS_UNAVAILABLE,
            f"the GitHub API did not answer within {GH_TIMEOUT_SECONDS}s",
        )
    except OSError as exc:
        return Probe(STATUS_UNAVAILABLE, f"the `gh` CLI could not be started: {exc}")

    if completed.returncode != 0:
        return Probe(
            STATUS_UNAVAILABLE,
            f"{SOURCE_REPOSITORY} is private and could not be read "
            f"({summarise_failure(completed)})",
        )

    try:
        payload: Any = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return Probe(STATUS_UNAVAILABLE, "the GitHub API returned a non-JSON body")
    if not isinstance(payload, list):
        return Probe(
            STATUS_UNAVAILABLE,
            f"{parent} did not list as a directory on {SOURCE_REF}",
        )

    entries = [item for item in payload if isinstance(item, dict) and item.get("name") == leaf]
    if not entries:
        return Probe(
            STATUS_DRIFTED,
            f"{SOURCE_PATH} no longer exists on {SOURCE_REPOSITORY}@{SOURCE_REF}; "
            "the skill was renamed, moved, or removed at the source",
        )
    if len(entries) > 1:
        return Probe(STATUS_UNAVAILABLE, f"{parent} listed {leaf} more than once")

    entry = entries[0]
    if entry.get("type") != "dir":
        return Probe(
            STATUS_DRIFTED,
            f"{SOURCE_PATH} is no longer a directory on "
            f"{SOURCE_REPOSITORY}@{SOURCE_REF} (type={entry.get('type')!r})",
        )
    tree_oid = entry.get("sha")
    if not isinstance(tree_oid, str) or not TREE_OID_RE.match(tree_oid):
        return Probe(STATUS_UNAVAILABLE, f"{parent} listed {leaf} without a usable tree OID")

    if tree_oid == SOURCE_TREE_OID:
        return Probe(STATUS_SYNCHRONISED, "the source tree still matches the copy", tree_oid)
    return Probe(
        STATUS_DRIFTED,
        f"{SOURCE_REPOSITORY}@{SOURCE_REF} moved ahead of the vendored copy",
        tree_oid,
    )


def report(probe: Probe, require_access: bool, stream_out, stream_err) -> int:
    if probe.status == STATUS_SYNCHRONISED:
        print(
            f"vendored skill source: OK {SOURCE_PATH} "
            f"{SOURCE_REPOSITORY}@{SOURCE_REF} tree={probe.tree_oid}",
            file=stream_out,
        )
        return EXIT_OK

    if probe.status == STATUS_UNAVAILABLE:
        print(
            f"vendored skill source: SKIPPED — {probe.detail}. The check needs a "
            f"token that can read {SOURCE_REPOSITORY}; without one the source "
            "cannot be compared and nothing is claimed about it.",
            file=stream_out if not require_access else stream_err,
        )
        return EXIT_UNAVAILABLE if require_access else EXIT_OK

    observed = probe.tree_oid or "<absent>"
    print(
        "vendored skill source: DRIFTED — the source moved ahead of the vendored "
        "copy and re-synchronisation is required.\n"
        f"  path:     {SOURCE_PATH}\n"
        f"  source:   {SOURCE_REPOSITORY}@{SOURCE_REF}\n"
        f"  expected: {SOURCE_TREE_OID} (recorded at {SOURCE_SHA})\n"
        f"  observed: {observed}\n"
        f"  detail:   {probe.detail}\n"
        "  action:   re-copy every file of the skill from the newer canonical "
        "commit, then update SOURCE_SHA and SOURCE_TREE_OID in "
        "scripts/check_vendored_skill_source.py together with SOURCE_SHA and "
        "EXPECTED_SKILL_DIGEST in tests/test_telegram_delivery_reporter.py — all "
        "in the same commit, so the recorded revision never names content other "
        "than the content checked in.",
        file=stream_err,
    )
    return EXIT_DRIFT


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="check_vendored_skill_source.py",
        description=(
            "Compare the recorded tree OID of the vendored "
            "telegram-delivery-reporter skill with the current source tree in "
            f"{SOURCE_REPOSITORY}."
        ),
    )
    parser.add_argument(
        "--require-source-access", action="store_true",
        help=(
            "fail with exit code 2 when the source cannot be read instead of "
            "reporting a skip; use it where a token with access is guaranteed"
        ),
    )
    try:
        arguments = parser.parse_args(argv)
    except SystemExit as exc:
        return EXIT_USAGE if exc.code else EXIT_OK
    return report(
        probe_source(run_gh), arguments.require_source_access, sys.stdout, sys.stderr
    )


if __name__ == "__main__":
    sys.exit(main())
