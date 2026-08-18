"""Offline tests for the vendored-skill source-drift detector.

`scripts/check_vendored_skill_source.py` is the only check in this repository
that reads the private controller repository, so it never joins the offline
gates. Its decision logic is still pure, and everything below exercises it
through an injected runner: no `gh`, no network, no token.

The suite also proves the recorded tree OID offline. A Git tree object names
only the entries directly inside it, so the tree OID of the vendored copy equals
the tree OID of the source directory exactly when the two hold the same bytes
and modes. Rebuilding that OID from the files on disk therefore pins the
detector's constant to the content that is actually checked in.
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[1]
DETECTOR_SCRIPT = ROOT / "scripts" / "check_vendored_skill_source.py"
REPORTER_TESTS = ROOT / "tests" / "test_telegram_delivery_reporter.py"
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
DRIFT_WORKFLOW = ROOT / ".github" / "workflows" / "vendored-skill-source.yml"

OTHER_TREE_OID = "5f2d3c4b" + "0" * 32


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


detector = load_module("check_vendored_skill_source", DETECTOR_SCRIPT)


def git(*args: str) -> bytes:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ).stdout


def tracked_entries(scope: str) -> list[tuple[str, str]]:
    """Return (mode, repository-relative path) for every tracked file of `scope`."""
    entries = []
    for record in git("ls-files", "-s", "-z", "--", scope).split(b"\0"):
        if not record:
            continue
        meta, path = record.split(b"\t", 1)
        mode, _blob, _stage = meta.split(b" ")
        entries.append((mode.decode("ascii"), path.decode("utf-8")))
    if not entries:
        raise RuntimeError(f"{scope} has no tracked files")
    return sorted(entries)


def _hash_tree(node: dict) -> bytes:
    """Hash one Git tree object built from a nested {name: entry} mapping.

    Git sorts tree entries by name, comparing a subtree as though its name
    carried a trailing slash, and stores each raw object ID as 20 bytes.
    """
    records = []
    for name, value in node.items():
        encoded = name.encode("utf-8")
        if isinstance(value, dict):
            records.append((encoded + b"/", b"40000 " + encoded + b"\0" + _hash_tree(value)))
        else:
            mode, digest = value
            records.append((encoded, mode.encode("ascii") + b" " + encoded + b"\0" + digest))
    payload = b"".join(record for _key, record in sorted(records))
    return hashlib.sha1(b"tree %d\0" % len(payload) + payload).digest()


def vendored_tree_oid(scope: str) -> str:
    """Rebuild the Git tree OID of `scope` from the tracked files on disk."""
    tree: dict = {}
    prefix = scope if scope.endswith("/") else scope + "/"
    for mode, path in tracked_entries(scope):
        parts = path[len(prefix):].split("/")
        node = tree
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        payload = (ROOT / path).read_bytes()
        blob = hashlib.sha1(b"blob %d\0" % len(payload) + payload).digest()
        node[parts[-1]] = (mode, blob)
    return _hash_tree(tree).hex()


def listing(sha: str | None, name: str | None = None, kind: str = "dir") -> str:
    """Render a GitHub `contents` listing of the skill's parent directory."""
    entries = [{"name": "ccm-delivery-executor", "type": "dir", "sha": "a" * 40}]
    if sha is not None:
        _parent, leaf = detector.parent_and_leaf(detector.SOURCE_PATH)
        entries.append({"name": name or leaf, "type": kind, "sha": sha})
    return json.dumps(entries)


def runner(stdout: str = "", stderr: str = "", returncode: int = 0):
    def run(argv):
        return subprocess.CompletedProcess(argv, returncode, stdout, stderr)
    return run


def raising_runner(error: BaseException):
    def run(argv):
        raise error
    return run


class RecordedTreeOidTests(unittest.TestCase):
    """The constant must name the content that is checked in, not a memory."""

    def test_recorded_tree_oid_is_the_vendored_copy_tree_oid(self):
        self.assertEqual(
            detector.SOURCE_TREE_OID,
            vendored_tree_oid(detector.SOURCE_PATH),
            "SOURCE_TREE_OID no longer matches the vendored copy on disk; the "
            "recorded source revision and the checked-in bytes must move "
            "together in one commit",
        )

    def test_recorded_tree_oid_is_the_git_tree_of_the_copy(self):
        """Cross-check the hand-rolled hasher against Git's own answer."""
        dirty = git("status", "--porcelain", "--untracked-files=no", "--",
                    detector.SOURCE_PATH).decode("utf-8")
        if dirty.strip():
            self.skipTest("skill copy differs from HEAD; nothing to cross-check")
        measured = git("rev-parse", f"HEAD:{detector.SOURCE_PATH}").decode("ascii").strip()
        self.assertEqual(measured, vendored_tree_oid(detector.SOURCE_PATH))
        self.assertEqual(measured, detector.SOURCE_TREE_OID)

    def test_detector_and_digest_gate_pin_the_same_source_revision(self):
        """One source of truth: both pins must name one repository and commit."""
        text = REPORTER_TESTS.read_text(encoding="utf-8")
        self.assertIn(f'SOURCE_SHA = "{detector.SOURCE_SHA}"', text)
        self.assertIn(f'SOURCE_REPOSITORY = "{detector.SOURCE_REPOSITORY}"', text)
        self.assertIn(f'SKILL_SCOPE = "{detector.SOURCE_PATH}/"', text)


class ProbeTests(unittest.TestCase):
    def probe(self, run):
        return detector.probe_source(run)

    def test_matching_tree_oid_is_synchronised(self):
        probe = self.probe(runner(stdout=listing(detector.SOURCE_TREE_OID)))
        self.assertEqual(detector.STATUS_SYNCHRONISED, probe.status)
        self.assertEqual(detector.SOURCE_TREE_OID, probe.tree_oid)

    def test_different_tree_oid_is_drift(self):
        probe = self.probe(runner(stdout=listing(OTHER_TREE_OID)))
        self.assertEqual(detector.STATUS_DRIFTED, probe.status)
        self.assertEqual(OTHER_TREE_OID, probe.tree_oid)

    def test_missing_skill_entry_is_drift(self):
        probe = self.probe(runner(stdout=listing(None)))
        self.assertEqual(detector.STATUS_DRIFTED, probe.status)
        self.assertIsNone(probe.tree_oid)
        self.assertIn("no longer exists", probe.detail)

    def test_renamed_skill_entry_is_drift(self):
        probe = self.probe(runner(stdout=listing(detector.SOURCE_TREE_OID, name="renamed")))
        self.assertEqual(detector.STATUS_DRIFTED, probe.status)

    def test_skill_replaced_by_a_file_is_drift(self):
        probe = self.probe(runner(stdout=listing(detector.SOURCE_TREE_OID, kind="file")))
        self.assertEqual(detector.STATUS_DRIFTED, probe.status)
        self.assertIn("no longer a directory", probe.detail)

    def test_failed_call_is_unavailable(self):
        probe = self.probe(runner(stderr="gh: Not Found (HTTP 404)", returncode=1))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)
        self.assertIn(detector.SOURCE_REPOSITORY, probe.detail)
        self.assertIn("404", probe.detail)

    def test_missing_gh_binary_is_unavailable(self):
        probe = self.probe(raising_runner(FileNotFoundError("gh")))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)
        self.assertIn("not installed", probe.detail)

    def test_timeout_is_unavailable(self):
        probe = self.probe(raising_runner(subprocess.TimeoutExpired(["gh"], 60)))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)
        self.assertIn("did not answer", probe.detail)

    def test_os_error_is_unavailable(self):
        probe = self.probe(raising_runner(OSError("permission denied")))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)

    def test_non_json_body_is_unavailable(self):
        probe = self.probe(runner(stdout="<html>login</html>"))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)

    def test_non_directory_body_is_unavailable(self):
        probe = self.probe(runner(stdout=json.dumps({"message": "Not Found"})))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)

    def test_malformed_tree_oid_is_unavailable(self):
        probe = self.probe(runner(stdout=listing("not-a-sha")))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)

    def test_duplicate_entries_are_unavailable(self):
        _parent, leaf = detector.parent_and_leaf(detector.SOURCE_PATH)
        body = json.dumps([
            {"name": leaf, "type": "dir", "sha": detector.SOURCE_TREE_OID},
            {"name": leaf, "type": "dir", "sha": OTHER_TREE_OID},
        ])
        probe = self.probe(runner(stdout=body))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)

    def test_failure_summary_never_echoes_a_token(self):
        secret = "ghp_" + "A1b2C3d4E5" * 4
        probe = self.probe(runner(stderr=f"bad credentials for {secret}", returncode=1))
        self.assertEqual(detector.STATUS_UNAVAILABLE, probe.status)
        self.assertNotIn(secret, probe.detail)
        self.assertIn("<redacted>", probe.detail)


class ReportTests(unittest.TestCase):
    def report(self, probe, require_access=False):
        out, err = io.StringIO(), io.StringIO()
        code = detector.report(probe, require_access, out, err)
        return code, out.getvalue(), err.getvalue()

    def test_synchronised_source_succeeds(self):
        probe = detector.Probe(detector.STATUS_SYNCHRONISED, "ok", detector.SOURCE_TREE_OID)
        code, out, err = self.report(probe)
        self.assertEqual(detector.EXIT_OK, code)
        self.assertIn("OK", out)
        self.assertEqual("", err)

    def test_drift_fails_and_names_both_tree_oids(self):
        probe = detector.Probe(detector.STATUS_DRIFTED, "moved ahead", OTHER_TREE_OID)
        code, _out, err = self.report(probe)
        self.assertEqual(detector.EXIT_DRIFT, code)
        self.assertIn(detector.SOURCE_TREE_OID, err)
        self.assertIn(OTHER_TREE_OID, err)
        self.assertIn(detector.SOURCE_SHA, err)
        self.assertIn("re-synchronisation is required", err)

    def test_drift_without_a_tree_oid_still_fails_readably(self):
        probe = detector.Probe(detector.STATUS_DRIFTED, "removed at the source")
        code, _out, err = self.report(probe)
        self.assertEqual(detector.EXIT_DRIFT, code)
        self.assertIn("<absent>", err)

    def test_unavailable_source_is_reported_as_a_skip(self):
        probe = detector.Probe(detector.STATUS_UNAVAILABLE, "no token")
        code, out, err = self.report(probe)
        self.assertEqual(detector.EXIT_OK, code)
        self.assertIn("SKIPPED", out)
        self.assertIn("nothing is claimed about it", out)
        self.assertEqual("", err)

    def test_unavailable_source_fails_when_access_is_required(self):
        probe = detector.Probe(detector.STATUS_UNAVAILABLE, "no token")
        code, out, err = self.report(probe, require_access=True)
        self.assertEqual(detector.EXIT_UNAVAILABLE, code)
        self.assertIn("SKIPPED", err)
        self.assertEqual("", out)


class CallShapeTests(unittest.TestCase):
    def test_the_check_makes_one_read_only_api_call(self):
        argv = detector.gh_argv()
        self.assertEqual(["gh", "api"], argv[:2])
        self.assertEqual(1, sum(1 for item in argv if item == "api"))
        for flag in ("-X", "--method", "-f", "-F", "--input"):
            self.assertNotIn(flag, argv)

    def test_the_call_lists_the_parent_of_the_skill_on_the_tracked_ref(self):
        parent, leaf = detector.parent_and_leaf(detector.SOURCE_PATH)
        self.assertEqual(
            f"repos/{detector.SOURCE_REPOSITORY}/contents/{parent}"
            f"?ref={detector.SOURCE_REF}",
            detector.gh_argv()[-1],
        )
        self.assertNotIn(leaf, detector.gh_argv()[-1])

    def test_no_credential_is_passed_on_the_command_line(self):
        for item in detector.gh_argv():
            self.assertNotIn("token", item.lower())
            self.assertNotIn("ghp_", item)


class WiringTests(unittest.TestCase):
    def test_the_network_check_stays_out_of_the_offline_gates(self):
        self.assertNotIn(DETECTOR_SCRIPT.name, CI_WORKFLOW.read_text(encoding="utf-8"))

    def test_the_offline_tests_run_in_the_governance_job(self):
        self.assertIn(Path(__file__).name, CI_WORKFLOW.read_text(encoding="utf-8"))

    def test_the_network_check_has_a_scheduled_workflow(self):
        workflow = DRIFT_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(f"scripts/{DETECTOR_SCRIPT.name}", workflow)
        self.assertIn("schedule:", workflow)
        self.assertIn("workflow_dispatch:", workflow)

    def test_the_offline_tests_need_no_network_module(self):
        source = Path(__file__).read_text(encoding="utf-8")
        for module in ("urllib", "http.client", "socket", "requests"):
            self.assertNotIn(f"import {module}", source)


if __name__ == "__main__":
    unittest.main()
