import copy
from datetime import datetime
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / ".agents/skills/ccm-delivery-executor/scripts/inspect_state.py"
SPEC = importlib.util.spec_from_file_location("inspect_state", SCRIPT)
inspector = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(inspector)


def run_git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.strip()


class ExecutionStateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        temporary_root = Path(self.temporary.name)
        self.root = temporary_root / "public"
        self.controller = temporary_root / "ccm-multi"
        self.root.mkdir(); self.controller.mkdir()
        for repository, remote in ((self.root, inspector.REMOTE), (self.controller, inspector.CONTROLLER_REMOTE)):
            run_git(repository, "init", "-b", "main")
            run_git(repository, "config", "user.email", "delivery-test@example.invalid")
            run_git(repository, "config", "user.name", "Delivery Test")
            run_git(repository, "remote", "add", "origin", remote)
        self.required_checks = ["Public capability manifest", "Rust 1.95 · linux-x86_64", "Rust 1.95 · macos-arm64"]
        self.acceptance = ["Socket permissions, cleanup, stale-socket detection, and cursor recovery are tested."]
        capability = {"document_type": "ccm-capability-manifest", "schema_version": 1,
            "repository": "https://github.com/korkin25/codex-claude-mode", "capabilities": [{
                "id": "ccm.serve.v1", "work_item": "CCM-SERVE-001", "status": "ready",
                "specification": "TODO.md#ccm-serve-001", "required_checks": self.required_checks}]}
        (self.root / "delivery").mkdir()
        (self.root / "delivery/capabilities.json").write_text(json.dumps(capability), encoding="utf-8")
        (self.root / "TODO.md").write_text("# Tasks\n\n### CCM-SERVE-001\n\n" + self.acceptance[0] + "\n", encoding="utf-8")
        (self.root / "base.txt").write_text("base\n", encoding="utf-8")
        run_git(self.root, "add", "."); run_git(self.root, "commit", "-m", "base")
        self.base = run_git(self.root, "rev-parse", "HEAD")
        run_git(self.root, "switch", "-c", "task/serve")
        self.evidence = {"id": "evidence-dependency", "kind": "merge_ci", "repository_id": "ccm-multi",
            "merge_sha": "d" * 40, "content_digest": "sha256:" + "e" * 64,
            "ci": {"provider": "github-actions", "run_id": "1", "url": "https://github.com/korkin25/ccm-multi/actions/runs/1",
                   "head_sha": "d" * 40, "status": "success", "required_checks": sorted(inspector.NORMATIVE_CHECKS["ccm-multi"])},
            "check_contract": None, "verified_at": "2026-01-01T00:00:00Z",
            "verifier": "controller", "provenance": "measured"}
        self.claim = {"id": "claim-serve-1", "work_item_id": "CCM-SERVE-001", "owner_principal": "agent-1",
            "repository_id": "ccm-public", "base_sha": self.base, "branch": "task/serve",
            "capabilities": ["ccm.public-client"], "generation": 1, "status": "active",
            "issued_at": "2026-01-01T00:00:00Z", "expires_at": "2030-01-01T00:00:00Z",
            "dependency_evidence_refs": ["evidence-dependency"]}
        self.controller_head = self.commit_controller([self.claim])
        self.state_path = Path("delivery/executions/claim-serve-1.json")

    def tearDown(self): self.temporary.cleanup()

    def controller_documents(self, claims):
        return {
            "claims.json": {"document_type": "claims-registry", "schema_version": 1, "claims": claims},
            "state.json": {"document_type": "delivery-state", "schema_version": 1,
                "repositories": [], "capabilities": [],
                "external_capabilities": [], "work_items": [
                    {"id": "CCM-REMOTE-CTL-001", "owner_repository": "ccm-multi",
                     "capabilities": ["multi.contracts"], "status": "done", "dependencies": [],
                     "external_prerequisites": [], "evidence_refs": ["evidence-dependency"]},
                    {"id": "CCM-SERVE-001", "owner_repository": "ccm-public",
                     "capabilities": ["ccm.public-client"], "status": "ready",
                     "dependencies": ["CCM-REMOTE-CTL-001"], "external_prerequisites": [], "evidence_refs": []}]},
            "evidence.json": {"document_type": "evidence-registry", "schema_version": 1,
                              "evidence": [self.evidence]},
        }

    def commit_controller(self, claims, mutate=None):
        directory = self.controller / "product/delivery"; directory.mkdir(parents=True, exist_ok=True)
        documents = self.controller_documents(claims)
        if mutate: mutate(documents)
        for name, document in documents.items():
            (directory / name).write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
        run_git(self.controller, "add", "."); run_git(self.controller, "commit", "-m", "controller snapshot")
        head = run_git(self.controller, "rev-parse", "HEAD")
        run_git(self.controller, "update-ref", "refs/remotes/origin/main", head)
        return head

    def state(self, **overrides):
        admission = {"controller_commit_sha": self.controller_head,
            "claim_digest": inspector.canonical_digest(self.claim), "claim_id": self.claim["id"],
            "claim_generation": self.claim["generation"], "predecessor": None,
            "owner_principal": self.claim["owner_principal"], "work_item_id": self.claim["work_item_id"],
            "public_capability_id": "ccm.serve.v1", "capabilities": self.claim["capabilities"],
            "base_sha": self.claim["base_sha"], "branch": self.claim["branch"],
            "issued_at": self.claim["issued_at"], "expires_at": self.claim["expires_at"],
            "dependency_evidence_refs": self.claim["dependency_evidence_refs"],
            "dependency_evidence": [{"id": self.evidence["id"], "digest": inspector.canonical_digest(self.evidence)}],
            "acceptance_digest": inspector.canonical_digest(self.acceptance),
            "required_checks": self.required_checks}
        admission.update(overrides)
        return {"document_type": "ccm-delivery-execution-state", "schema_version": 1,
            "repository": {"id": "ccm-public", "remote": inspector.REMOTE, "default_branch": "main"},
            "admission": admission,
            "execution": {"phase": "claimed", "completed_acceptance": [],
                          "remaining_work": ["Implement acceptance criteria"],
                          "completed_checks": [{"name": self.required_checks[0], "command": "read TODO", "outcome": "passed"}]},
            "checkpoint": {"sequence": 1, "kind": "claim", "parent_sha": self.base,
                           "updated_at": "2026-01-01T00:01:00Z", "next_action": "Implement the task"}}

    def commit_state(self, state, path=None):
        path = path or self.state_path; target = self.root / path; target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
        run_git(self.root, "add", str(path)); run_git(self.root, "commit", "-m", "checkpoint")
        return run_git(self.root, "rev-parse", "HEAD")

    def inspect(self, state, remote_head, *, controller=True, at="2026-08-17T00:00:00Z", path=None):
        return inspector.inspect(self.root, path or self.state_path, state,
            datetime.fromisoformat(at.replace("Z", "+00:00")), remote_head,
            self.controller if controller else None, self.controller_head if controller else None)

    def test_clean_checkpoint_requires_and_accepts_controller_snapshot(self):
        state = self.state(); head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))
        local = self.inspect(state, head, controller=False)
        self.assertEqual(("local_only", False), (local["classification"], local["admitted"]))

    def test_schema_and_phase_kind_table_are_strict(self):
        self.assertEqual([], inspector.validate_state(self.state()))
        schema = json.loads((ROOT / ".agents/skills/ccm-delivery-executor/references/execution-state.schema.json").read_text())
        self.assertEqual(set(inspector.TOP_KEYS), set(schema["required"]))
        self.assertEqual(set(inspector.ADMISSION_KEYS), set(schema["properties"]["admission"]["required"]))
        for phase, kind in inspector.PHASE_KIND.items():
            state = self.state(); state["execution"]["phase"] = phase; state["checkpoint"]["kind"] = kind
            if phase == "candidate": state["execution"]["remaining_work"] = []
            self.assertNotIn("PHASE_KIND_MISMATCH", inspector.validate_state(state))
            state["checkpoint"]["kind"] = "blocked" if kind != "blocked" else "progress"
            self.assertIn("PHASE_KIND_MISMATCH", inspector.validate_state(state))

    def test_candidate_rejects_failed_check_and_remaining_work(self):
        state = self.state(); state["execution"]["phase"] = "candidate"; state["checkpoint"]["kind"] = "candidate"
        state["execution"]["completed_checks"].append({"name": self.required_checks[1], "command": "test", "outcome": "failed"})
        errors = inspector.validate_state(state)
        self.assertIn("CANDIDATE_REMAINING_WORK", errors); self.assertIn("CANDIDATE_FAILED_CHECK", errors)

    def test_state_bytes_and_index_flags_are_checked_without_status(self):
        state = self.state(); head = self.commit_state(state)
        run_git(self.root, "update-index", "--assume-unchanged", str(self.state_path))
        target = self.root / self.state_path; target.write_text(target.read_text() + " ", encoding="utf-8")
        result = self.inspect(state, head)
        self.assertEqual("invalid", result["classification"])
        self.assertIn("STATE_BYTES_DIFFER_FROM_HEAD", result["errors"])
        self.assertIn("STATE_INDEX_FLAG_FORBIDDEN", result["errors"])

    def test_skip_worktree_state_is_rejected_even_when_bytes_match(self):
        state = self.state(); head = self.commit_state(state)
        run_git(self.root, "update-index", "--skip-worktree", str(self.state_path))
        result = self.inspect(state, head)
        self.assertIn("STATE_INDEX_FLAG_FORBIDDEN", result["errors"])

    def test_state_mode_must_be_regular_non_executable(self):
        state = self.state(); self.commit_state(state)
        run_git(self.root, "update-index", "--chmod=+x", str(self.state_path)); run_git(self.root, "commit", "-m", "bad mode")
        head = run_git(self.root, "rev-parse", "HEAD")
        result = self.inspect(state, head)
        self.assertIn("STATE_HEAD_MODE_OR_ENTRY_MISMATCH", result["errors"])

    def test_https_multiple_push_url_and_rewrite_are_rejected(self):
        state = self.state(); head = self.commit_state(state)
        run_git(self.root, "config", "--add", "remote.origin.url", "https://github.com/korkin25/codex-claude-mode")
        result = self.inspect(state, head)
        self.assertTrue(any("FETCH_URL_MISMATCH" in error for error in result["errors"]))
        run_git(self.root, "config", "--unset-all", "remote.origin.url")
        run_git(self.root, "config", "--add", "remote.origin.url", inspector.REMOTE)
        run_git(self.root, "config", "--add", "remote.origin.pushurl", inspector.REMOTE)
        run_git(self.root, "config", "--add", "remote.origin.pushurl", "https://github.com/korkin25/codex-claude-mode")
        result = self.inspect(state, head)
        self.assertTrue(any("PUSH_URL_MISMATCH" in error for error in result["errors"]))
        run_git(self.root, "config", "--unset-all", "remote.origin.pushurl")
        run_git(self.root, "config", "url.git@evil.invalid:.insteadOf", "git@github.com:")
        result = self.inspect(state, head)
        self.assertIn("LOCAL_GIT_ACTIVE_CONFIG_FORBIDDEN", result["errors"])

    def test_dirty_stale_diverged_and_expired_are_fail_closed(self):
        state = self.state(); head = self.commit_state(state)
        (self.root / "dirty.txt").write_text("dirty", encoding="utf-8")
        self.assertEqual("dirty", self.inspect(state, head)["classification"])
        (self.root / "dirty.txt").unlink()
        self.assertEqual("stale", self.inspect(state, self.base)["classification"])
        run_git(self.root, "switch", "-c", "remote-side", self.base)
        (self.root / "remote.txt").write_text("remote", encoding="utf-8"); run_git(self.root, "add", "."); run_git(self.root, "commit", "-m", "remote")
        remote = run_git(self.root, "rev-parse", "HEAD"); run_git(self.root, "switch", "task/serve")
        self.assertEqual("diverged", self.inspect(state, remote)["classification"])
        self.assertEqual("expired_claim", self.inspect(state, head, at="2031-01-01T00:00:00Z")["classification"])

    def test_controller_revocation_supersession_and_evidence_tamper_fail(self):
        state = self.state(); head = self.commit_state(state)
        revoked = copy.deepcopy(self.claim); revoked["status"] = "revoked"
        self.controller_head = self.commit_controller([revoked])
        result = self.inspect(state, head)
        self.assertIn("CONTROLLER_CLAIM_REVOKED_OR_CHANGED", result["errors"])
        later = copy.deepcopy(self.claim); later.update({"id": "claim-serve-later", "generation": 2,
                                                         "status": "expired"})
        self.controller_head = self.commit_controller([self.claim, later])
        result = self.inspect(state, head)
        self.assertIn("CONTROLLER_CLAIM_SUPERSEDED", result["errors"])
        self.evidence["content_digest"] = "sha256:" + "f" * 64
        self.controller_head = self.commit_controller([self.claim])
        result = self.inspect(state, head)
        self.assertIn("EVIDENCE_DIGEST_MISMATCH evidence-dependency", result["errors"])

    def test_controller_duplicate_json_is_local_only(self):
        state = self.state(); head = self.commit_state(state)
        path = self.controller / "product/delivery/claims.json"
        path.write_text('{"document_type":"claims-registry","document_type":"claims-registry","schema_version":1,"claims":[]}')
        run_git(self.controller, "add", str(path)); run_git(self.controller, "commit", "-m", "duplicate")
        self.controller_head = run_git(self.controller, "rev-parse", "HEAD")
        run_git(self.controller, "update-ref", "refs/remotes/origin/main", self.controller_head)
        result = self.inspect(state, head)
        self.assertEqual("local_only", result["classification"])
        self.assertTrue(any("DUPLICATE_KEY" in error for error in result["errors"]))

    def test_generation_one_requires_base_parent(self):
        (self.root / "extra.txt").write_text("extra", encoding="utf-8")
        run_git(self.root, "add", "."); run_git(self.root, "commit", "-m", "unclaimed work")
        parent = run_git(self.root, "rev-parse", "HEAD")
        state = self.state(); state["checkpoint"]["parent_sha"] = parent
        head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertIn("GENERATION_ONE_PARENT_NOT_BASE", result["errors"])

    def test_generation_two_explicitly_chains_closed_predecessor(self):
        first = self.state(); predecessor_head = self.commit_state(first)
        closed = copy.deepcopy(self.claim); closed["status"] = "expired"
        second_claim = copy.deepcopy(self.claim); second_claim.update({"id": "claim-serve-2", "generation": 2})
        self.claim = second_claim
        self.controller_head = self.commit_controller([closed, second_claim])
        second_path = Path("delivery/executions/claim-serve-2.json")
        second = self.state(predecessor={"claim_id": "claim-serve-1", "generation": 1,
                                         "checkpoint_sha": predecessor_head})
        second["checkpoint"]["parent_sha"] = predecessor_head
        second["checkpoint"]["updated_at"] = "2026-01-01T00:02:00Z"
        head = self.commit_state(second, second_path)
        result = self.inspect(second, head, path=second_path)
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))

    def test_generation_two_rejects_mismatched_predecessor_admission(self):
        first = self.state(); predecessor_head = self.commit_state(first)
        closed = copy.deepcopy(self.claim); closed.update({"status": "expired", "owner_principal": "different-agent"})
        second_claim = copy.deepcopy(self.claim); second_claim.update({"id": "claim-serve-2", "generation": 2})
        self.claim = second_claim; self.controller_head = self.commit_controller([closed, second_claim])
        second_path = Path("delivery/executions/claim-serve-2.json")
        second = self.state(predecessor={"claim_id": "claim-serve-1", "generation": 1,
                                         "checkpoint_sha": predecessor_head})
        second["checkpoint"].update({"parent_sha": predecessor_head, "updated_at": "2026-01-01T00:02:00Z"})
        head = self.commit_state(second, second_path)
        result = self.inspect(second, head, path=second_path)
        self.assertIn("PREDECESSOR_ADMISSION_IDENTITY_MISMATCH", result["errors"])

    def test_timestamps_and_append_only_history_are_enforced(self):
        first = self.state(); first["execution"]["completed_acceptance"] = self.acceptance
        first_head = self.commit_state(first)
        second = copy.deepcopy(first); second["checkpoint"].update({"sequence": 2, "kind": "progress",
            "parent_sha": first_head, "updated_at": "2026-01-01T00:01:00Z"})
        second["execution"].update({"phase": "implementing", "completed_acceptance": [], "completed_checks": []})
        head = self.commit_state(second)
        result = self.inspect(second, head)
        self.assertIn("COMPLETED_ACCEPTANCE_NOT_APPEND_ONLY", result["errors"])
        self.assertIn("COMPLETED_CHECKS_NOT_APPEND_ONLY", result["errors"])
        self.assertIn("CHECKPOINT_TIME_NOT_MONOTONIC", result["errors"])

    def test_checkpoint_from_future_is_rejected(self):
        state = self.state(); state["checkpoint"]["updated_at"] = "2027-01-01T00:00:00Z"
        head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertIn("CHECKPOINT_FROM_FUTURE", result["errors"])

    def test_injected_git_config_environment_is_ignored(self):
        state = self.state(); head = self.commit_state(state)
        injected = {"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "remote.origin.url",
                    "GIT_CONFIG_VALUE_0": "https://attacker.invalid/repository",
                    "GIT_SSH_COMMAND": "false", "GIT_INDEX_FILE": "/dev/null",
                    "GIT_NAMESPACE": "attacker", "GIT_SHALLOW_FILE": "/dev/null"}
        with mock.patch.dict("os.environ", injected, clear=False):
            result = self.inspect(state, head)
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))
        self.assertTrue(Path(inspector.git_command(("version",))[0]).is_absolute())
        self.assertNotIn("GIT_INDEX_FILE", inspector.git_environment())

    def test_active_git_config_is_rejected_before_attacker_program_runs(self):
        state = self.state(); head = self.commit_state(state)
        sentinel = self.root.parent / "executed"
        attacker = self.root.parent / "attacker"
        attacker.write_text(f"#!/bin/sh\ntouch '{sentinel}'\n", encoding="utf-8"); attacker.chmod(0o755)
        run_git(self.root, "config", "core.fsmonitor", str(attacker))
        result = self.inspect(state, head)
        self.assertIn("LOCAL_GIT_ACTIVE_CONFIG_FORBIDDEN", result["errors"])
        self.assertFalse(sentinel.exists())

    def test_shallow_replace_and_split_index_metadata_fail_before_probes(self):
        state = self.state(); head = self.commit_state(state)
        (self.root / ".git/shallow").write_text(self.base + "\n", encoding="utf-8")
        result = self.inspect(state, head)
        self.assertIn("GIT_ALTERNATE_SHALLOW_OR_SPARSE_STATE_FORBIDDEN", result["errors"])

    def test_controller_requires_fixed_root_and_prefetched_origin_main(self):
        state = self.state(); head = self.commit_state(state)
        run_git(self.controller, "commit", "--allow-empty", "-m", "unmeasured controller change")
        advanced = run_git(self.controller, "rev-parse", "HEAD")
        run_git(self.controller, "update-ref", "refs/remotes/origin/main", advanced)
        result = self.inspect(state, head)
        self.assertEqual("local_only", result["classification"])
        self.assertIn("CONTROLLER_PREFETCHED_ORIGIN_MAIN_MISMATCH", result["errors"])

    def test_arbitrary_controller_root_is_rejected(self):
        state = self.state(); head = self.commit_state(state)
        arbitrary = self.root.parent / "arbitrary"; arbitrary.mkdir()
        result = inspector.inspect(self.root, self.state_path, state,
            datetime.fromisoformat("2026-08-17T00:00:00+00:00"), head, arbitrary, self.controller_head)
        self.assertEqual("local_only", result["classification"])
        self.assertIn("CONTROLLER_ROOT_NONCANONICAL", result["errors"])

    def test_evidence_requires_canonical_ci_contract(self):
        self.evidence["ci"]["url"] = "https://github.com/other/project/actions/runs/1"
        self.evidence["ci"]["required_checks"] = ["invented"]
        self.evidence["check_contract"] = {"path": "delivery/capabilities.json",
            "format": "ccm-capability-manifest-v1", "content_digest": "sha256:" + "a" * 64}
        self.controller_head = self.commit_controller([self.claim])
        state = self.state(); head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertIn("EVIDENCE_CI_URL controller.evidence.evidence-dependency", result["errors"])
        self.assertIn("EVIDENCE_REQUIRED_CHECKS_NONNORMATIVE controller.evidence.evidence-dependency", result["errors"])
        self.assertIn("EVIDENCE_CHECK_CONTRACT_UNEXPECTED controller.evidence.evidence-dependency", result["errors"])

    def test_candidate_is_bound_to_exact_acceptance_and_manifest_checks(self):
        state = self.state(); state["execution"].update({"phase": "candidate", "remaining_work": [],
            "completed_acceptance": self.acceptance,
            "completed_checks": [{"name": name, "command": f"verify {name}", "outcome": "passed"}
                                 for name in self.required_checks]})
        state["checkpoint"]["kind"] = "candidate"
        self.assertEqual([], inspector.public_contract_errors(self.root, self.base, state))
        state["execution"]["completed_acceptance"] = ["caller supplied"]
        state["execution"]["completed_checks"] = [{"name": "invented", "command": "true", "outcome": "passed"}]
        errors = inspector.public_contract_errors(self.root, self.base, state)
        self.assertIn("COMPLETED_ACCEPTANCE_NOT_NORMATIVE_PREFIX", errors)
        self.assertIn("CANDIDATE_ACCEPTANCE_INCOMPLETE", errors)
        self.assertIn("COMPLETED_CHECK_NOT_NORMATIVE", errors)
        self.assertIn("CANDIDATE_REQUIRED_CHECKS_INCOMPLETE", errors)


if __name__ == "__main__": unittest.main()
