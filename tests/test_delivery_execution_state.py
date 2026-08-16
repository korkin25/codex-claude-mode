import copy
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / ".agents/skills/ccm-delivery-executor/scripts/inspect_state.py"
SPEC = importlib.util.spec_from_file_location("inspect_state", SCRIPT)
inspector = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(inspector)


def run_git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, check=True, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ).stdout.strip()


class ExecutionStateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        run_git(self.root, "init", "-b", "main")
        run_git(self.root, "config", "user.email", "delivery-test@example.invalid")
        run_git(self.root, "config", "user.name", "Delivery Test")
        run_git(self.root, "remote", "add", "origin", inspector.REMOTE)
        (self.root / "base.txt").write_text("base\n", encoding="utf-8")
        run_git(self.root, "add", "base.txt")
        run_git(self.root, "commit", "-m", "base")
        self.base = run_git(self.root, "rev-parse", "HEAD")
        run_git(self.root, "switch", "-c", "task/serve")
        self.state_path = Path("delivery/executions/claim-serve-1.json")

    def tearDown(self):
        self.temporary.cleanup()

    def state(self, *, expires_at="2030-01-01T00:00:00Z"):
        return {
            "document_type": "ccm-delivery-execution-state",
            "schema_version": 1,
            "repository": {
                "id": "ccm-public",
                "remote": inspector.REMOTE,
                "default_branch": "main",
            },
            "admission": {
                "controller_commit_sha": "a" * 40,
                "claim_digest": "sha256:" + "b" * 64,
                "claim_id": "claim-serve-1",
                "claim_generation": 1,
                "owner_principal": "agent-1",
                "work_item_id": "CCM-SERVE-001",
                "capability_id": "ccm.serve.v1",
                "base_sha": self.base,
                "branch": "task/serve",
                "issued_at": "2026-01-01T00:00:00Z",
                "expires_at": expires_at,
                "dependency_evidence_refs": [],
            },
            "execution": {
                "phase": "claimed",
                "completed_acceptance": [],
                "remaining_work": ["Implement acceptance criteria"],
                "completed_checks": [],
            },
            "checkpoint": {
                "sequence": 1,
                "kind": "claim",
                "parent_sha": self.base,
                "updated_at": "2026-01-01T00:01:00Z",
                "next_action": "Read the claimed acceptance criteria",
            },
        }

    def commit_state(self, state):
        path = self.root / self.state_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
        run_git(self.root, "add", str(self.state_path))
        run_git(self.root, "commit", "-m", "checkpoint")
        return run_git(self.root, "rev-parse", "HEAD")

    def inspect(self, state, remote_head, at="2026-08-17T00:00:00Z"):
        return inspector.inspect(
            self.root, self.state_path, state,
            datetime.fromisoformat(at.replace("Z", "+00:00")), remote_head,
        )

    def test_valid_state_and_schema_are_strict_json(self):
        self.assertEqual([], inspector.validate_state(self.state()))
        schema = json.loads(
            (ROOT / ".agents/skills/ccm-delivery-executor/references/execution-state.schema.json")
            .read_text(encoding="utf-8")
        )
        self.assertFalse(schema["additionalProperties"])
        self.assertEqual(set(inspector.TOP_KEYS), set(schema["required"]))

    def test_invalid_state_fails_closed(self):
        state = self.state()
        state["execution"]["phase"] = "merged"
        state["admission"]["branch"] = "../unsafe"
        errors = inspector.validate_state(state)
        self.assertIn("FORMAT execution.phase", errors)
        self.assertIn("INVALID_BRANCH admission.branch", errors)

    def test_clean_checkpoint_is_admitted(self):
        state = self.state()
        head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertEqual("clean", result["classification"])
        self.assertTrue(result["admitted"])

    def test_dirty_checkpoint_is_not_admitted(self):
        state = self.state()
        head = self.commit_state(state)
        (self.root / "untracked.txt").write_text("work\n", encoding="utf-8")
        result = self.inspect(state, head)
        self.assertEqual("dirty", result["classification"])
        self.assertFalse(result["admitted"])

    def test_expired_claim_is_not_admitted(self):
        state = self.state(expires_at="2026-02-01T00:00:00Z")
        head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertEqual("expired_claim", result["classification"])

    def test_unpushed_local_checkpoint_is_stale(self):
        state = self.state()
        self.commit_state(state)
        result = self.inspect(state, self.base)
        self.assertEqual("stale", result["classification"])
        self.assertIn("LOCAL_CHECKPOINT_NOT_PUSHED", result["errors"])

    def test_state_not_updated_in_head_is_stale(self):
        state = self.state()
        self.commit_state(state)
        (self.root / "later.txt").write_text("later\n", encoding="utf-8")
        run_git(self.root, "add", "later.txt")
        run_git(self.root, "commit", "-m", "uncheckpointed commit")
        head = run_git(self.root, "rev-parse", "HEAD")
        result = self.inspect(state, head)
        self.assertEqual("stale", result["classification"])
        self.assertIn("STATE_NOT_UPDATED_IN_HEAD", result["errors"])

    def test_diverged_local_and_remote_are_rejected(self):
        run_git(self.root, "switch", "-c", "remote-side", self.base)
        (self.root / "remote.txt").write_text("remote\n", encoding="utf-8")
        run_git(self.root, "add", "remote.txt")
        run_git(self.root, "commit", "-m", "remote side")
        remote_head = run_git(self.root, "rev-parse", "HEAD")
        run_git(self.root, "switch", "task/serve")
        state = self.state()
        self.commit_state(state)
        result = self.inspect(state, remote_head)
        self.assertEqual("diverged", result["classification"])
        self.assertFalse(result["admitted"])

    def test_candidate_cannot_have_remaining_work(self):
        state = self.state()
        state["execution"]["phase"] = "candidate"
        state["checkpoint"]["kind"] = "candidate"
        self.assertIn("CANDIDATE_REMAINING_WORK", inspector.validate_state(state))

    def test_checkpoint_sequence_and_bindings_are_append_only(self):
        first = self.state()
        first_head = self.commit_state(first)
        second = copy.deepcopy(first)
        second["admission"]["base_sha"] = "c" * 40
        second["checkpoint"]["parent_sha"] = first_head
        second["checkpoint"]["sequence"] = 3
        second["checkpoint"]["kind"] = "progress"
        second["execution"]["phase"] = "implementing"
        head = self.commit_state(second)
        result = self.inspect(second, head)
        self.assertEqual("stale", result["classification"])
        self.assertIn("IMMUTABLE_BINDING_CHANGED", result["errors"])
        self.assertIn("CHECKPOINT_SEQUENCE_NOT_MONOTONIC", result["errors"])


if __name__ == "__main__":
    unittest.main()
