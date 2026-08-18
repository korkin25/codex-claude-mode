import copy
from datetime import datetime
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/git-config"
SCRIPT = ROOT / ".agents/skills/ccm-delivery-executor/scripts/inspect_state.py"
PREFLIGHT = ROOT / ".agents/skills/ccm-delivery-executor/scripts/preflight_inspector.py"
SPEC = importlib.util.spec_from_file_location("inspect_state", SCRIPT)
inspector = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(inspector)
PREFLIGHT_SPEC = importlib.util.spec_from_file_location("preflight_inspector", PREFLIGHT)
preflight = importlib.util.module_from_spec(PREFLIGHT_SPEC)
assert PREFLIGHT_SPEC.loader is not None
PREFLIGHT_SPEC.loader.exec_module(preflight)


def run_git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.strip()


class ExecutionStateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        # macOS exposes its temporary root through /var -> /private/var. Build
        # fixture repositories from the canonical path just like production.
        temporary_root = Path(self.temporary.name).resolve()
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
                "specification": "TODO.md#ccm-serve-001", "content_scope": ["src/", "tests/"],
                "required_checks": self.required_checks}]}
        (self.root / "delivery").mkdir()
        base_inspector = self.root / ".agents/skills/ccm-delivery-executor/scripts/inspect_state.py"
        base_inspector.parent.mkdir(parents=True)
        base_inspector.write_bytes(SCRIPT.read_bytes())
        self.manifest_raw = json.dumps(capability).encode()
        self.todo_raw = ("# Tasks\n\n### CCM-SERVE-001\n\n" + self.acceptance[0] + "\n").encode()
        (self.root / "delivery/capabilities.json").write_bytes(self.manifest_raw)
        (self.root / "TODO.md").write_bytes(self.todo_raw)
        (self.root / "base.txt").write_text("base\n", encoding="utf-8")
        run_git(self.root, "add", "."); run_git(self.root, "commit", "-m", "base")
        self.base = run_git(self.root, "rev-parse", "HEAD")
        run_git(self.root, "update-ref", "refs/remotes/origin/main", self.base)
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
                "repositories": [
                    {"id": "ccm-public", "role": "public_client",
                     "url": "https://github.com/korkin25/codex-claude-mode", "baseline_sha": self.base},
                    {"id": "ccm-multi", "role": "product_integration",
                     "url": "https://github.com/korkin25/ccm-multi", "baseline_sha": "c" * 40},
                    {"id": "aor", "role": "authoritative_runtime",
                     "url": "https://github.com/korkin25/agent-orchestrator", "baseline_sha": "a" * 40}],
                "capabilities": [
                    {"id": "ccm.public-client", "owner_repository": "ccm-public",
                     "write_exclusive": True, "scope": "public client"},
                    {"id": "multi.contracts", "owner_repository": "ccm-multi",
                     "write_exclusive": True, "scope": "shared contracts"}],
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
        capability = json.loads(self.manifest_raw)["capabilities"][0]
        contract = {"manifest_digest": inspector.bytes_digest(self.manifest_raw),
                    "todo_digest": inspector.bytes_digest(self.todo_raw),
                    "acceptance": self.acceptance, "capability": capability}
        # The admission binds the identity fields only; bounded-lane scoping fields a
        # registry record may additionally carry are deliberately outside the digest.
        identity = {key: self.claim[key] for key in inspector.CLAIM_KEYS}
        admission = {"controller_commit_sha": self.controller_head,
            "claim_digest": inspector.canonical_digest(identity), "claim_id": self.claim["id"],
            "claim_generation": self.claim["generation"], "predecessor": None,
            "owner_principal": self.claim["owner_principal"], "work_item_id": self.claim["work_item_id"],
            "public_capability_id": "ccm.serve.v1", "capabilities": self.claim["capabilities"],
            "base_sha": self.claim["base_sha"], "branch": self.claim["branch"],
            "issued_at": self.claim["issued_at"], "expires_at": self.claim["expires_at"],
            "dependency_evidence_refs": self.claim["dependency_evidence_refs"],
            "dependency_evidence": [{"id": self.evidence["id"], "digest": inspector.canonical_digest(self.evidence)}],
            "acceptance_digest": inspector.canonical_digest(self.acceptance),
            "contract_digest": inspector.canonical_digest(contract),
            "inspector_digest": inspector.bytes_digest(SCRIPT.read_bytes()),
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

    def generation_three(self, *, second_predecessor_id="claim-serve-1", first_state_mutate=None):
        first_state = self.state()
        if first_state_mutate:
            first_state_mutate(first_state)
        first_head = self.commit_state(first_state)
        first_claim = copy.deepcopy(self.claim); first_claim["status"] = "released"

        second_claim = copy.deepcopy(self.claim)
        second_claim.update({"id": "claim-serve-2", "generation": 2})
        self.claim = second_claim
        self.controller_head = self.commit_controller([first_claim, second_claim])
        second_path = Path("delivery/executions/claim-serve-2.json")
        second_state = self.state(predecessor={"claim_id": second_predecessor_id,
            "generation": 1, "checkpoint_sha": first_head})
        second_state["checkpoint"].update({"parent_sha": first_head,
            "updated_at": "2026-01-01T00:02:00Z"})
        second_head = self.commit_state(second_state, second_path)

        second_claim["status"] = "revoked"
        third_claim = copy.deepcopy(second_claim)
        third_claim.update({"id": "claim-serve-3", "generation": 3, "status": "active"})
        self.claim = third_claim
        self.controller_head = self.commit_controller([first_claim, second_claim, third_claim])
        third_path = Path("delivery/executions/claim-serve-3.json")
        third_state = self.state(predecessor={"claim_id": "claim-serve-2",
            "generation": 2, "checkpoint_sha": second_head})
        third_state["checkpoint"].update({"parent_sha": second_head,
            "updated_at": "2026-01-01T00:03:00Z"})
        third_head = self.commit_state(third_state, third_path)
        return third_state, third_path, third_head, first_claim, second_claim, third_claim

    def inspect(self, state, remote_head, *, controller=True, at="2026-08-17T00:00:00Z", path=None,
                public_main_head=None, predecessor_remote_head=None):
        predecessor_head = state["admission"]["predecessor"]
        predecessor_head = predecessor_head["checkpoint_sha"] if predecessor_head else None
        if predecessor_remote_head is None: predecessor_remote_head = predecessor_head
        return inspector.inspect(self.root, path or self.state_path, state,
            datetime.fromisoformat(at.replace("Z", "+00:00")), remote_head,
            self.controller if controller else None, self.controller_head if controller else None,
            self.base if public_main_head is None else public_main_head, predecessor_remote_head,
            inspector.bytes_digest(SCRIPT.read_bytes()))

    def test_public_main_requires_exact_external_prefetch_and_admission_binding(self):
        state = self.state(); head = self.commit_state(state)
        self.assertIn("PUBLIC_MAIN_MEASUREMENT_REQUIRED",
                      inspector.inspect(self.root, self.state_path, state,
                        datetime.fromisoformat("2026-08-17T00:00:00+00:00"), head)["errors"])
        unknown = "f" * 40
        self.assertIn("PUBLIC_MAIN_MEASURED_HEAD_UNKNOWN",
                      self.inspect(state, head, public_main_head=unknown)["errors"])
        run_git(self.root, "update-ref", "-d", "refs/remotes/origin/main")
        self.assertIn("PUBLIC_PREFETCHED_ORIGIN_MAIN_MISSING", self.inspect(state, head)["errors"])
        run_git(self.root, "update-ref", "refs/remotes/origin/main", head)
        self.assertIn("PUBLIC_PREFETCHED_ORIGIN_MAIN_MISMATCH", self.inspect(state, head)["errors"])
        self.assertIn("PUBLIC_MAIN_BASE_MISMATCH",
                      self.inspect(state, head, public_main_head=head)["errors"])

    def test_contract_is_base_bound_and_scope_covers_entire_diff(self):
        state = self.state(); state["admission"]["contract_digest"] = "sha256:" + "0" * 64
        head = self.commit_state(state)
        self.assertIn("CONTRACT_DIGEST_MISMATCH", self.inspect(state, head)["errors"])
        run_git(self.root, "reset", "--soft", "HEAD^")
        (self.root / "TODO.md").write_text("tampered\n", encoding="utf-8")
        run_git(self.root, "add", "."); run_git(self.root, "commit", "-m", "tamper governance")
        head = run_git(self.root, "rev-parse", "HEAD")
        self.assertIn("CONTENT_SCOPE_VIOLATION TODO.md", self.inspect(state, head)["errors"])

    def test_inspector_is_bound_to_external_digest(self):
        state = self.state(inspector_digest="sha256:" + "0" * 64); head = self.commit_state(state)
        self.assertIn("INSPECTOR_EXTERNAL_DIGEST_MISMATCH", self.inspect(state, head)["errors"])

    def test_preflight_executes_authenticated_base_inspector_not_task_tree_copy(self):
        state = self.state(); head = self.commit_state(state)
        fixture_script = self.root / ".agents/skills/ccm-delivery-executor/scripts/inspect_state.py"
        sentinel = self.root.parent / "substituted-inspector-executed"
        fixture_script.write_text(
            f"from pathlib import Path\nPath({str(sentinel)!r}).write_text('executed')\n",
            encoding="utf-8",
        )
        run_git(self.root, "add", str(fixture_script.relative_to(self.root)))
        run_git(self.root, "commit", "-m", "substitute task-tree inspector")
        substituted_head = run_git(self.root, "rev-parse", "HEAD")
        digest = inspector.bytes_digest(SCRIPT.read_bytes())
        arguments = ["inspect", "--state", str(self.state_path),
            "--at", "2026-08-17T00:00:00Z", "--remote-head", substituted_head,
            "--public-main-head", self.base, "--inspector-digest", digest,
            "--controller-root", str(self.controller), "--controller-head", self.controller_head]
        status = preflight.run_authenticated_inspector(self.root, self.base, digest, arguments)
        self.assertNotEqual(0, status)
        self.assertFalse(sentinel.exists())

    def test_preflight_rejects_all_downstream_root_spellings(self):
        digest = inspector.bytes_digest(SCRIPT.read_bytes())
        variants = (["--root", str(self.root)], [f"--root={self.root}"],
                    ["--roo", str(self.root)], [f"--roo={self.root}"],
                    ["--root", str(self.root), "--root", str(self.root)])
        for injected in variants:
            with self.subTest(arguments=injected):
                with self.assertRaisesRegex(preflight.PreflightError,
                                            "PREFLIGHT_RESERVED_ROOT_ARGUMENT"):
                    preflight.run_authenticated_inspector(
                        self.root, self.base, digest, ["inspect", *injected]
                    )

    def test_preflight_rejects_alternate_clone_with_same_origin(self):
        alternate = self.root.parent / "alternate-public"
        subprocess.run(["git", "clone", "--no-local", str(self.root), str(alternate)],
                       check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        run_git(alternate, "remote", "set-url", "origin", inspector.REMOTE)
        arguments = ["--task-root", str(alternate), "--base-sha", self.base,
                     "--inspector-digest", inspector.bytes_digest(SCRIPT.read_bytes()),
                     "--", "inspect", "--state", str(self.state_path)]
        with mock.patch.object(preflight, "configured_task_root", return_value=self.root), \
                mock.patch("sys.stdout"):
            status = preflight.main(arguments)
        self.assertEqual(1, status)

    def root_config(self, raw: str):
        config = self.root.parent / "delivery-executor.json"
        config.write_text(raw, encoding="utf-8")
        config.chmod(0o600)
        return mock.patch.object(preflight, "owner_root_config", return_value=config)

    def test_preflight_root_config_rejects_duplicate_canonical_root(self):
        raw = (f'{{"schema_version":1,"canonical_root":{json.dumps(str(self.root))},'
               f'"canonical_root":{json.dumps(str(self.controller))}}}')
        with self.root_config(raw), self.assertRaisesRegex(
                preflight.PreflightError, "PREFLIGHT_ROOT_CONFIG_DUPLICATE_KEY canonical_root"):
            preflight.configured_task_root()

    def test_preflight_root_config_rejects_duplicate_schema_version(self):
        raw = (f'{{"schema_version":1,"schema_version":2,'
               f'"canonical_root":{json.dumps(str(self.root))}}}')
        with self.root_config(raw), self.assertRaisesRegex(
                preflight.PreflightError, "PREFLIGHT_ROOT_CONFIG_DUPLICATE_KEY schema_version"):
            preflight.configured_task_root()

    def test_preflight_root_config_rejects_boolean_schema_versions(self):
        for value in ("true", "false"):
            with self.subTest(value=value):
                raw = (f'{{"schema_version":{value},'
                       f'"canonical_root":{json.dumps(str(self.root))}}}')
                with self.root_config(raw), self.assertRaisesRegex(
                        preflight.PreflightError, "PREFLIGHT_ROOT_CONFIG_INVALID"):
                    preflight.configured_task_root()

    def test_preflight_root_config_accepts_integer_schema_version_one(self):
        raw = (f'{{"schema_version":1,'
               f'"canonical_root":{json.dumps(str(self.root))}}}')
        with self.root_config(raw):
            self.assertEqual(self.root, preflight.configured_task_root())

    def test_inspector_parser_rejects_abbreviated_root(self):
        arguments = ["inspect", "--roo", str(self.root), "--state", str(self.state_path),
                     "--at", "2026-08-17T00:00:00Z", "--remote-head", self.base,
                     "--public-main-head", self.base, "--inspector-digest",
                     inspector.bytes_digest(SCRIPT.read_bytes())]
        stderr = io.StringIO()
        with mock.patch("sys.stderr", stderr), self.assertRaises(SystemExit):
            inspector.main(arguments)
        self.assertIn("the following arguments are required: --root", stderr.getvalue())

    def test_cli_preserves_lexical_symlink_root(self):
        state = self.state(); head = self.commit_state(state)
        alias = self.root.parent / "public-cli-alias"; alias.symlink_to(self.root, target_is_directory=True)
        with mock.patch("sys.stdout"):
            status = inspector.main(["inspect", "--root", str(alias), "--state", str(self.state_path),
                "--at", "2026-08-17T00:00:00Z", "--remote-head", head,
                "--public-main-head", self.base, "--inspector-digest", inspector.bytes_digest(SCRIPT.read_bytes())])
        self.assertEqual(1, status)

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

    def test_all_tracked_schema_versions_require_exact_integer_one(self):
        for value in (True, False):
            with self.subTest(document="execution-state", value=value):
                state = self.state(); state["schema_version"] = value
                self.assertIn("SCHEMA_VERSION", inspector.validate_state(state))

            for filename in ("claims.json", "state.json", "evidence.json"):
                with self.subTest(document=filename, value=value):
                    documents = self.controller_documents([self.claim])
                    documents[filename]["schema_version"] = value
                    errors = []
                    inspector.validate_controller_snapshot(
                        documents, datetime.fromisoformat("2026-08-17T00:00:00+00:00"),
                        "schema-test", errors,
                    )
                    self.assertIn(
                        "CONTROLLER_SCHEMA_VERSION schema-test",
                        errors,
                    )

    def test_all_tracked_schema_versions_accept_exact_integer_one(self):
        state = self.state()
        self.assertIs(type(state["schema_version"]), int)
        self.assertEqual(1, state["schema_version"])
        for document in self.controller_documents([self.claim]).values():
            self.assertIs(type(document["schema_version"]), int)
            self.assertEqual(1, document["schema_version"])
        head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))

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
        self.assertIn("EXPIRED_CLAIM_TIME_INVALID controller.claim.claim-serve-later", result["errors"])
        self.evidence["content_digest"] = "sha256:" + "f" * 64
        self.controller_head = self.commit_controller([self.claim])
        result = self.inspect(state, head)
        self.assertIn("EVIDENCE_DIGEST_MISMATCH evidence-dependency", result["errors"])

    def test_evidence_added_after_issuance_cannot_retroactively_authorize_claim(self):
        self.controller_head = self.commit_controller(
            [self.claim],
            lambda documents: documents["evidence.json"].update({"evidence": []}),
        )
        state = self.state()
        head = self.commit_state(state)
        self.controller_head = self.commit_controller([self.claim])

        result = self.inspect(state, head)
        self.assertIn(
            "CONTROLLER_ISSUANCE_EVIDENCE_MISSING generation-1 evidence-dependency",
            result["errors"],
        )

    def test_unrelated_terminal_claim_with_nonexistent_references_is_rejected(self):
        unrelated = copy.deepcopy(self.claim)
        unrelated.update({
            "id": "claim-unrelated", "work_item_id": "CCM-GHOST-001",
            "repository_id": "ghost", "capabilities": ["ghost.capability"],
            "status": "released", "dependency_evidence_refs": ["evidence-ghost"],
        })
        state = self.state()
        head = self.commit_state(state)
        self.controller_head = self.commit_controller([self.claim, unrelated])

        result = self.inspect(state, head)
        self.assertIn(
            "CONTROLLER_CLAIM_WORK_MISSING controller.claim.claim-unrelated",
            result["errors"],
        )
        self.assertIn(
            "CONTROLLER_CLAIM_EVIDENCE_MISSING controller.claim.claim-unrelated evidence-ghost",
            result["errors"],
        )

    def test_historical_state_evidence_digest_is_checked_at_its_issuance(self):
        def corrupt_binding(state):
            state["admission"]["dependency_evidence"][0]["digest"] = "sha256:" + "f" * 64

        state, path, head, *_ = self.generation_three(first_state_mutate=corrupt_binding)
        result = self.inspect(state, head, path=path)
        self.assertIn(
            "CONTROLLER_ISSUANCE_EVIDENCE_DIGEST_MISMATCH generation-1 evidence-dependency",
            result["errors"],
        )

    def test_blocked_issuance_cannot_be_retroactively_made_ready(self):
        def blocked_at_issuance(documents):
            work_items = documents["state.json"]["work_items"]
            next(item for item in work_items if item["id"] == "CCM-SERVE-001")["status"] = "blocked"
            next(item for item in work_items if item["id"] == "CCM-REMOTE-CTL-001")["status"] = "blocked"

        self.controller_head = self.commit_controller([self.claim], blocked_at_issuance)
        state = self.state()
        head = self.commit_state(state)
        self.controller_head = self.commit_controller([self.claim])

        result = self.inspect(state, head)
        self.assertIn("CONTROLLER_ISSUANCE_WORK_NOT_READY generation-1", result["errors"])
        self.assertIn(
            "CONTROLLER_ISSUANCE_DEPENDENCY_NOT_DONE generation-1 CCM-REMOTE-CTL-001",
            result["errors"],
        )

    def test_evidence_verified_after_claim_issuance_is_rejected(self):
        self.evidence["verified_at"] = "2026-01-01T00:00:01Z"
        self.controller_head = self.commit_controller([self.claim])
        state = self.state()
        head = self.commit_state(state)

        result = self.inspect(state, head)
        self.assertIn(
            "CONTROLLER_ISSUANCE_EVIDENCE_AFTER_CLAIM generation-1 evidence-dependency",
            result["errors"],
        )

    def test_dependency_evidence_owner_and_type_are_bound_at_issuance(self):
        self.evidence["kind"] = "external_probe"

        def wrong_dependency_owner(documents):
            dependency = next(
                item for item in documents["state.json"]["work_items"]
                if item["id"] == "CCM-REMOTE-CTL-001"
            )
            dependency["owner_repository"] = "aor"

        self.controller_head = self.commit_controller([self.claim], wrong_dependency_owner)
        state = self.state()
        head = self.commit_state(state)

        result = self.inspect(state, head)
        self.assertIn(
            "CONTROLLER_ISSUANCE_DEPENDENCY_EVIDENCE_OWNER_MISMATCH "
            "generation-1 CCM-REMOTE-CTL-001 evidence-dependency",
            result["errors"],
        )
        self.assertIn(
            "CONTROLLER_ISSUANCE_DEPENDENCY_EVIDENCE_TYPE_MISMATCH "
            "generation-1 CCM-REMOTE-CTL-001 evidence-dependency",
            result["errors"],
        )

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
        closed = copy.deepcopy(self.claim); closed["status"] = "released"
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
        tree = run_git(self.root, "show", "-s", "--format=%T", predecessor_head)
        newer = run_git(self.root, "commit-tree", tree, "-p", predecessor_head, "-m", "newer predecessor checkpoint")
        result = self.inspect(second, head, path=second_path, predecessor_remote_head=newer)
        self.assertIn("PREDECESSOR_NOT_LATEST_REMOTE_HEAD", result["errors"])

    def test_generation_three_recursively_binds_all_exact_state_paths(self):
        state, path, head, *_ = self.generation_three()
        result = self.inspect(state, head, path=path)
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]), result)

    def test_generation_three_rejects_broken_historical_link(self):
        state, path, head, *_ = self.generation_three(second_predecessor_id="claim-wrong-1")
        result = self.inspect(state, head, path=path)
        self.assertIn("LINEAGE_PREDECESSOR_LINK_MISMATCH 1", result["errors"])

    def test_generation_three_gap_and_fork_fail_end_to_end(self):
        state, path, head, first, second, third = self.generation_three()
        self.controller_head = self.commit_controller([first, third])
        gap = self.inspect(state, head, path=path)
        self.assertIn("CONTROLLER_CLAIM_HISTORY_INCOMPLETE 2", gap["errors"])
        sibling = copy.deepcopy(second); sibling["id"] = "claim-serve-2-sibling"
        self.controller_head = self.commit_controller([first, second, sibling, third])
        fork = self.inspect(state, head, path=path)
        self.assertIn("CONTROLLER_CLAIM_GENERATION_FORK 2", fork["errors"])

    def test_invalid_historical_claim_status_enum_fails_end_to_end(self):
        first = self.state(); first_head = self.commit_state(first)
        invalid = copy.deepcopy(self.claim); invalid["status"] = "closed"
        second = copy.deepcopy(self.claim); second.update({"id": "claim-serve-2", "generation": 2})
        self.claim = second; self.controller_head = self.commit_controller([invalid, second])
        second_path = Path("delivery/executions/claim-serve-2.json")
        state = self.state(predecessor={"claim_id": invalid["id"], "generation": 1,
                                        "checkpoint_sha": first_head})
        state["checkpoint"].update({"parent_sha": first_head, "updated_at": "2026-01-01T00:02:00Z"})
        head = self.commit_state(state, second_path)
        result = self.inspect(state, head, path=second_path)
        self.assertIn("FORMAT controller.claim.claim-serve-1.status", result["errors"])
        self.assertIn("CONTROLLER_PREDECESSOR_NOT_TERMINAL 1", result["errors"])

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

    def test_controller_rejects_same_generation_claim_fork_end_to_end(self):
        sibling = copy.deepcopy(self.claim)
        sibling.update({"id": "claim-serve-sibling", "status": "expired"})
        self.controller_head = self.commit_controller([self.claim, sibling])
        state = self.state(); head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertIn("CONTROLLER_CLAIM_GENERATION_FORK 1", result["errors"])

    def test_claim_history_requires_complete_unique_chain_and_exact_predecessor(self):
        first = copy.deepcopy(self.claim); first["status"] = "expired"
        second = copy.deepcopy(self.claim); second.update({"id": "claim-serve-2", "generation": 2,
                                                            "status": "expired"})
        third = copy.deepcopy(self.claim); third.update({"id": "claim-serve-3", "generation": 3})
        indexed = {item["id"]: item for item in (first, second, third)}
        self.assertEqual([], inspector.claim_history_errors(
            indexed, third, {"claim_id": second["id"], "generation": 2, "checkpoint_sha": "a" * 40}))
        wrong = inspector.claim_history_errors(
            indexed, third, {"claim_id": first["id"], "generation": 2, "checkpoint_sha": "a" * 40})
        self.assertIn("PREDECESSOR_CLAIM_ID_MISMATCH", wrong)
        gap = inspector.claim_history_errors(
            {first["id"]: first, third["id"]: third}, third,
            {"claim_id": first["id"], "generation": 2, "checkpoint_sha": "a" * 40})
        self.assertIn("CONTROLLER_CLAIM_HISTORY_INCOMPLETE 2", gap)

    def test_controller_rejects_capability_alias_in_claim_lineage(self):
        alias = copy.deepcopy(self.claim)
        alias.update({"id": "claim-serve-alias", "capabilities": ["ccm.other"], "status": "expired"})
        self.controller_head = self.commit_controller([self.claim, alias])
        state = self.state(); head = self.commit_state(state)
        result = self.inspect(state, head)
        self.assertIn("CONTROLLER_CLAIM_LINEAGE_CAPABILITIES_MISMATCH claim-serve-alias", result["errors"])

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
        command = inspector.git_command(("version",))
        self.assertTrue(Path(command[0]).is_absolute())
        self.assertIn("core.fsmonitor=false", command)
        self.assertFalse(any("core.hooksPath" in argument for argument in command))
        self.assertNotIn("GIT_INDEX_FILE", inspector.git_environment())

    def test_linux_and_macos_local_config_fixtures_are_parsed_without_git(self):
        expected = [inspector.LocalConfigEntry("core.repositoryformatversion", "0"),
                    inspector.LocalConfigEntry("core.filemode", "true"),
                    inspector.LocalConfigEntry("remote.origin.url", inspector.REMOTE),
                    inspector.LocalConfigEntry("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")]
        for platform in ("linux", "macos"):
            with self.subTest(platform=platform):
                raw = (FIXTURES / f"{platform}.config").read_bytes()
                if platform == "macos":
                    raw = raw.replace(b"\n", b"\r\n")
                with mock.patch.object(inspector, "bounded_git", side_effect=AssertionError("config parser invoked Git")):
                    self.assertEqual(expected, inspector.parse_local_config(raw))

    def test_local_config_parser_rejects_executable_and_ambiguous_syntax(self):
        active = b'[core]\n\tfsmonitor = /tmp/attacker\n'
        parsed = inspector.parse_local_config(active)
        self.assertEqual("core.fsmonitor", parsed[0].key)
        for raw in (b'[include]\npath = /tmp/attacker\n',
                    b'[remote "origin"]\nurl = value\\\ncontinued\n',
                    b'[remote "origin"]\nurl = "unterminated\n',
                    b'[core]\nkey = value\\escape\n'):
            with self.subTest(raw=raw):
                if raw.startswith(b"[include]"):
                    entries = inspector.parse_local_config(raw)
                    self.assertEqual("include.path", entries[0].key)
                else:
                    with self.assertRaises(ValueError):
                        inspector.parse_local_config(raw)

    def test_repository_guard_still_rejects_a_symlink_root(self):
        alias = self.root.parent / "public-alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.assertIn("REPOSITORY_ROOT_NOT_CANONICAL", inspector.repository_guard(alias))

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
            datetime.fromisoformat("2026-08-17T00:00:00+00:00"), head, arbitrary, self.controller_head,
            self.base, None, inspector.bytes_digest(SCRIPT.read_bytes()))
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
        digest = inspector.bytes_digest(SCRIPT.read_bytes())
        self.assertEqual([], inspector.public_contract_errors(self.root, self.base, self.state_path, state, digest))
        state["execution"]["completed_acceptance"] = ["caller supplied"]
        state["execution"]["completed_checks"] = [{"name": "invented", "command": "true", "outcome": "passed"}]
        errors = inspector.public_contract_errors(self.root, self.base, self.state_path, state, digest)
        self.assertIn("COMPLETED_ACCEPTANCE_NOT_NORMATIVE_PREFIX", errors)
        self.assertIn("CANDIDATE_ACCEPTANCE_INCOMPLETE", errors)
        self.assertIn("COMPLETED_CHECK_NOT_NORMATIVE", errors)
        self.assertIn("CANDIDATE_REQUIRED_CHECKS_INCOMPLETE", errors)

    def scope_claim(self, **overrides):
        self.claim = copy.deepcopy(self.claim)
        self.claim.update({"write_paths": ["src/**"], "content_scopes": ["ccm.serve.v1"]})
        self.claim.update(overrides)
        return self.claim

    def second_public_lane(self, documents):
        documents["state.json"]["capabilities"].append(
            {"id": "ccm.public-docs", "owner_repository": "ccm-public",
             "write_exclusive": True, "scope": "public documentation"})
        documents["state.json"]["work_items"].append(
            {"id": "CCM-DOCS-001", "owner_repository": "ccm-public",
             "capabilities": ["ccm.public-docs"], "status": "ready", "dependencies": [],
             "external_prerequisites": [], "evidence_refs": []})

    def public_lane(self, **overrides):
        record = {"id": "claim-docs-1", "work_item_id": "CCM-DOCS-001",
            "owner_principal": "agent-docs", "repository_id": "ccm-public",
            "base_sha": self.base, "branch": "task/docs",
            "capabilities": ["ccm.public-docs"], "generation": 1, "status": "active",
            "issued_at": "2026-01-01T00:00:00Z", "expires_at": "2030-01-01T00:00:00Z",
            "dependency_evidence_refs": []}
        record.update(overrides)
        return record

    def controller_lane(self, **overrides):
        record = {"id": "claim-multi-1", "work_item_id": "CCM-REMOTE-CTL-001",
            "owner_principal": "agent-contracts", "repository_id": "ccm-multi",
            "base_sha": "c" * 40, "branch": "task/contracts",
            "capabilities": ["multi.contracts"], "generation": 1, "status": "active",
            "issued_at": "2026-01-01T00:00:00Z", "expires_at": "2030-01-01T00:00:00Z",
            "dependency_evidence_refs": [],
            "write_paths": ["product/contracts/**"], "content_scopes": ["shared.protocol"]}
        record.update(overrides)
        return record

    def inspect_registry(self, claims, mutate=None):
        run_git(self.root, "reset", "--hard", self.base)
        self.controller_head = self.commit_controller(claims, mutate)
        state = self.state()
        return self.inspect(state, self.commit_state(state))

    def test_bounded_lane_scope_fields_are_optional_and_admitted(self):
        result = self.inspect_registry([self.scope_claim()])
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))

    def test_disjoint_parallel_lane_in_another_repository_is_admitted(self):
        result = self.inspect_registry([self.scope_claim(), self.controller_lane()])
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))

    def test_second_active_lane_in_owner_repository_is_rejected(self):
        for label, other in (
            ("scoped", self.public_lane(write_paths=["docs/**"],
                                        content_scopes=["ccm.public-docs"])),
            ("unscoped", self.public_lane()),
        ):
            with self.subTest(other=label):
                result = self.inspect_registry([self.scope_claim(), other],
                                               self.second_public_lane)
                self.assertFalse(result["admitted"])
                self.assertIn(
                    "CONTROLLER_ACTIVE_CLAIM_CONFLICT claim-docs-1 repository=ccm-public",
                    result["errors"])

    def test_overlapping_write_path_between_lanes_is_rejected(self):
        result = self.inspect_registry(
            [self.scope_claim(), self.public_lane(write_paths=["src/**"],
                                                  content_scopes=["ccm.public-docs"])],
            self.second_public_lane)
        self.assertFalse(result["admitted"])
        self.assertIn(
            "CONTROLLER_ACTIVE_CLAIM_CONFLICT claim-docs-1 repository=ccm-public,path=src/**",
            result["errors"])

    def test_shared_content_scope_between_lanes_is_rejected(self):
        result = self.inspect_registry(
            [self.scope_claim(), self.controller_lane(content_scopes=["ccm.serve.v1"])])
        self.assertFalse(result["admitted"])
        self.assertIn(
            "CONTROLLER_ACTIVE_CLAIM_CONFLICT claim-multi-1 content_scope=ccm.serve.v1",
            result["errors"])

    def test_revoked_claim_is_still_rejected_as_inactive_lane(self):
        result = self.inspect_registry([self.scope_claim(status="revoked")])
        self.assertFalse(result["admitted"])
        self.assertIn("CONTROLLER_ACTIVE_CLAIM_CONFLICT", result["errors"])
        self.assertIn("CONTROLLER_CLAIM_REVOKED_OR_CHANGED", result["errors"])

    def test_unknown_claim_key_is_still_rejected(self):
        result = self.inspect_registry([self.scope_claim(lane="extra")])
        self.assertFalse(result["admitted"])
        self.assertIn("UNKNOWN controller.claim.claim-serve-1: lane", result["errors"])

    def test_malformed_lane_scopes_are_rejected(self):
        where = "controller.claim.claim-serve-1"
        for label, overrides, expected in (
            ("order", {"write_paths": ["src/**", "Cargo.toml"]}, f"ORDER {where}.write_paths"),
            ("traversal", {"write_paths": ["../etc/passwd"]}, f"FORMAT {where}.write_paths[0]"),
            ("wildcard", {"write_paths": ["src/*.rs"]}, f"FORMAT {where}.write_paths[0]"),
            ("empty-paths", {"write_paths": []},
             f"TYPE {where}.write_paths: expected non-empty array"),
            ("empty-scopes", {"content_scopes": []},
             f"TYPE {where}.content_scopes: expected non-empty array"),
            ("scope-format", {"content_scopes": ["Not An Id"]},
             f"FORMAT {where}.content_scopes[0]"),
        ):
            with self.subTest(case=label):
                result = self.inspect_registry([self.scope_claim(**overrides)])
                self.assertFalse(result["admitted"])
                self.assertIn(expected, result["errors"])

    def test_claim_conflict_rules_mirror_controller_bounded_lanes(self):
        for left, right, overlap in (
            ("src/**", "src/main.rs", True),
            ("src/**", "src", True),
            ("src/**", "srcx/main.rs", False),
            ("src/a/**", "src/**", True),
            ("Cargo.toml", "Cargo.toml", True),
            ("Cargo.toml", "Cargo.lock", False),
        ):
            with self.subTest(left=left, right=right):
                self.assertEqual(overlap, inspector.paths_overlap(left, right))
                self.assertEqual(overlap, inspector.paths_overlap(right, left))
        ours = {"repository_id": "ccm-public", "work_item_id": "CCM-SERVE-001",
                "capabilities": ["ccm.public-client"], "write_paths": ["src/**"],
                "content_scopes": ["ccm.serve.v1"]}
        elsewhere = {"repository_id": "ccm-multi", "work_item_id": "CCM-REMOTE-CTL-001",
                     "capabilities": ["multi.contracts"], "write_paths": ["src/**"],
                     "content_scopes": ["shared.protocol"]}
        # Identical paths in a different repository are a different file tree.
        self.assertEqual([], inspector.claim_conflict_reasons(ours, elsewhere))
        self.assertEqual(["content_scope=ccm.serve.v1"], inspector.claim_conflict_reasons(
            ours, {**elsewhere, "content_scopes": ["ccm.serve.v1"]}))
        self.assertEqual(["work_item=CCM-SERVE-001"], inspector.claim_conflict_reasons(
            ours, {**elsewhere, "work_item_id": "CCM-SERVE-001"}))
        self.assertEqual(["capability=ccm.public-client"], inspector.claim_conflict_reasons(
            ours, {**elsewhere, "capabilities": ["ccm.public-client"]}))
        self.assertEqual(["repository=ccm-public"], inspector.claim_conflict_reasons(
            ours, {**elsewhere, "repository_id": "ccm-public", "write_paths": ["docs/**"],
                   "content_scopes": ["ccm.public-docs"]}))
        self.assertEqual(["repository=ccm-public", "path=src/**"],
                         inspector.claim_conflict_reasons(
                             ours, {**elsewhere, "repository_id": "ccm-public",
                                    "content_scopes": ["ccm.public-docs"]}))
        # A record issued before scoping existed claims its whole repository.
        self.assertEqual(["repository=ccm-public"], inspector.claim_conflict_reasons(
            ours, {"repository_id": "ccm-public", "work_item_id": "CCM-DOCS-001",
                   "capabilities": ["ccm.public-docs"]}))

    def aor_baseline(self, index, *, lineage=True, **overrides):
        """One AOR baseline evidence record shaped exactly like the controller writes it."""
        token = "0123456789abcdef"[index]
        record = {"id": f"evidence-aor-baseline-{token * 7}", "kind": "merge_ci",
            "repository_id": "aor", "merge_sha": token * 40,
            "content_digest": "sha256:" + token * 64,
            "ci": {"provider": "github-actions", "run_id": str(index + 1),
                   "url": f"https://github.com/korkin25/agent-orchestrator/actions/runs/{index + 1}",
                   "head_sha": token * 40, "status": "success",
                   "required_checks": sorted(inspector.NORMATIVE_CHECKS["aor"])},
            "check_contract": None, "verified_at": "2026-01-01T00:00:00Z",
            "verifier": "controller", "provenance": "measured"}
        if lineage:
            supersedes = None if index == 0 else f"evidence-aor-baseline-{'0123456789abcdef'[index - 1] * 7}"
            record["lineage"] = {"type": "aor_baseline", "generation": index + 1,
                                 "supersedes": supersedes}
        record.update(overrides)
        return record

    def with_evidence(self, *records):
        def mutate(documents):
            documents["evidence.json"]["evidence"].extend(copy.deepcopy(records))
        return mutate

    def baseline_where(self, index):
        return f"controller.evidence.evidence-aor-baseline-{'0123456789abcdef'[index] * 7}"

    def test_aor_baseline_lineage_chain_is_admitted(self):
        # The live ccm-multi registry carries one AOR baseline chain that grows by one
        # generation per refresh. Only its shape is pinned here: binding the test to the
        # current baseline SHAs would break on the next refresh without a real defect.
        for generations in (1, 2, 3):
            with self.subTest(generations=generations):
                chain = [self.aor_baseline(index) for index in range(generations)]
                result = self.inspect_registry([self.claim], self.with_evidence(*chain))
                self.assertEqual(("clean", True),
                                 (result["classification"], result["admitted"]))

    def test_baseline_evidence_without_lineage_is_still_admitted(self):
        # The registry is append-only and keeps baselines recorded before the lineage.
        result = self.inspect_registry(
            [self.claim], self.with_evidence(self.aor_baseline(0, lineage=False)))
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))

    def test_unknown_key_inside_lineage_is_rejected(self):
        record = self.aor_baseline(0)
        record["lineage"]["note"] = "annotation"
        result = self.inspect_registry([self.claim], self.with_evidence(record))
        self.assertFalse(result["admitted"])
        self.assertIn(f"UNKNOWN {self.baseline_where(0)}.lineage: note", result["errors"])

    def test_incomplete_lineage_triple_is_rejected(self):
        record = self.aor_baseline(0)
        del record["lineage"]["supersedes"]
        result = self.inspect_registry([self.claim], self.with_evidence(record))
        self.assertFalse(result["admitted"])
        self.assertIn(f"MISSING {self.baseline_where(0)}.lineage: supersedes", result["errors"])

    def test_malformed_lineage_fields_are_rejected(self):
        where = f"{self.baseline_where(0)}.lineage"
        for label, lineage, expected in (
            ("type", {"type": "aor-baseline"}, f"EVIDENCE_LINEAGE_TYPE {where}"),
            ("unknown-type", {"type": "public_baseline"}, f"EVIDENCE_LINEAGE_TYPE {where}"),
            ("generation-string", {"generation": "1"}, f"FORMAT {where}.generation"),
            ("generation-boolean", {"generation": True}, f"FORMAT {where}.generation"),
            ("generation-zero", {"generation": 0}, f"FORMAT {where}.generation"),
            ("generation-float", {"generation": 1.0}, f"FORMAT {where}.generation"),
            ("supersedes-integer", {"generation": 2, "supersedes": 1},
             f"FORMAT {where}.supersedes"),
            ("supersedes-format", {"generation": 2, "supersedes": "aor-baseline"},
             f"FORMAT {where}.supersedes"),
            ("lineage-not-object", None, f"TYPE {where}: expected object"),
        ):
            with self.subTest(case=label):
                record = self.aor_baseline(0)
                if lineage is None:
                    record["lineage"] = "aor_baseline"
                else:
                    record["lineage"].update(lineage)
                result = self.inspect_registry([self.claim], self.with_evidence(record))
                self.assertFalse(result["admitted"])
                self.assertIn(expected, result["errors"])

    def test_lineage_origin_must_match_generation_one(self):
        where = f"{self.baseline_where(0)}.lineage"
        for label, lineage in (
            ("first-generation-supersedes", {"generation": 1,
                                             "supersedes": "evidence-aor-baseline-9999999"}),
            ("later-generation-opens-chain", {"generation": 2, "supersedes": None}),
        ):
            with self.subTest(case=label):
                record = self.aor_baseline(0)
                record["lineage"].update(lineage)
                result = self.inspect_registry([self.claim], self.with_evidence(record))
                self.assertFalse(result["admitted"])
                self.assertIn(f"EVIDENCE_LINEAGE_ORIGIN {where}", result["errors"])

    def test_lineage_outside_aor_baseline_evidence_is_rejected(self):
        for label, overrides in (
            ("foreign-repository", {"repository_id": "ccm-multi"}),
            ("foreign-id", {"id": "evidence-aor-owner-0000000"}),
        ):
            with self.subTest(case=label):
                record = self.aor_baseline(0, **overrides)
                result = self.inspect_registry([self.claim], self.with_evidence(record))
                self.assertFalse(result["admitted"])
                self.assertIn(
                    f"EVIDENCE_LINEAGE_SCOPE controller.evidence.{record['id']}",
                    result["errors"])

    def test_lineage_chain_rejects_fork_gap_dangling_and_disorder(self):
        first, second, third = (self.aor_baseline(index) for index in range(3))
        sibling = copy.deepcopy(second)
        sibling["id"] = "evidence-aor-baseline-2222222"
        gap = copy.deepcopy(third)
        gap["lineage"]["supersedes"] = None
        dangling = copy.deepcopy(second)
        dangling["lineage"]["supersedes"] = "evidence-aor-baseline-9999999"
        disorder = copy.deepcopy(third)
        disorder["lineage"]["supersedes"] = first["id"]
        for label, records, expected in (
            ("fork", (first, second, sibling),
             f"CONTROLLER_EVIDENCE_LINEAGE_FORK controller aor_baseline generation=2 "
             f"ids={second['id']},{sibling['id']}"),
            ("gap", (first, gap),
             "CONTROLLER_EVIDENCE_LINEAGE_GAP controller aor_baseline generations=1,3"),
            ("dangling", (first, dangling),
             f"CONTROLLER_EVIDENCE_LINEAGE_MISSING {self.baseline_where(1)} "
             "evidence-aor-baseline-9999999"),
            ("disorder", (first, second, disorder),
             f"CONTROLLER_EVIDENCE_LINEAGE_ORDER {self.baseline_where(2)} "
             f"actual={first['id']} expected={second['id']}"),
        ):
            with self.subTest(case=label):
                result = self.inspect_registry([self.claim], self.with_evidence(*records))
                self.assertFalse(result["admitted"])
                self.assertIn(expected, result["errors"])

    def historical_evidence(self, repository_id, merge_sha, checks, **overrides):
        record = {"id": "evidence-historical", "kind": "merge_ci",
            "repository_id": repository_id, "merge_sha": merge_sha,
            "content_digest": "sha256:" + "a" * 64,
            "ci": {"provider": "github-actions", "run_id": "7",
                   "url": f"{inspector.REPOSITORY_WEB[repository_id]}/actions/runs/7",
                   "head_sha": merge_sha, "status": "success",
                   "required_checks": sorted(checks)},
            "check_contract": None, "verified_at": "2026-01-01T00:00:00Z",
            "verifier": "controller", "provenance": "measured"}
        record.update(overrides)
        return record

    def evidence_errors(self, record):
        errors = []
        inspector.validate_evidence(
            record, datetime.fromisoformat("2026-08-17T00:00:00+00:00"), "evidence", errors)
        return errors

    def test_pre_contract_merge_keeps_the_checks_its_own_workflow_declared(self):
        for repository_id, merge_sha in sorted(inspector.HISTORICAL_CHECKS):
            checks = inspector.HISTORICAL_CHECKS[(repository_id, merge_sha)]
            with self.subTest(repository=repository_id, merge_sha=merge_sha[:7]):
                self.assertEqual([], self.evidence_errors(
                    self.historical_evidence(repository_id, merge_sha, checks)))
                # The identical reduced set on any other merge is not normative: a check
                # name comes from the workflow, so only that exact tree lacked the job.
                self.assertIn("EVIDENCE_REQUIRED_CHECKS_NONNORMATIVE evidence",
                              self.evidence_errors(
                                  self.historical_evidence(repository_id, "f" * 40, checks)))

    def test_pre_manifest_merge_may_carry_no_check_contract(self):
        for repository_id, merge_sha in sorted(inspector.HISTORICAL_UNCONTRACTED):
            with self.subTest(merge_sha=merge_sha[:7]):
                checks = inspector.HISTORICAL_CHECKS[(repository_id, merge_sha)]
                self.assertEqual([], self.evidence_errors(
                    self.historical_evidence(repository_id, merge_sha, checks)))
        # Every later merge of an owner repository must still bind its manifest.
        self.assertIn("EVIDENCE_CHECK_CONTRACT_REQUIRED evidence", self.evidence_errors(
            self.historical_evidence("ccm-public", "f" * 40,
                                     inspector.NORMATIVE_CHECKS["ccm-public"])))

    def test_historical_check_tables_only_narrow_the_current_contract(self):
        for (repository_id, merge_sha), checks in inspector.HISTORICAL_CHECKS.items():
            with self.subTest(repository=repository_id, merge_sha=merge_sha[:7]):
                self.assertRegex(merge_sha, r"^[0-9a-f]{40}$")
                # A pre-contract merge may only lack checks, never invent one.
                self.assertLess(frozenset(checks),
                                frozenset(inspector.NORMATIVE_CHECKS[repository_id]))
        self.assertLessEqual(inspector.HISTORICAL_UNCONTRACTED, set(inspector.HISTORICAL_CHECKS))
        for repository_id, _ in inspector.HISTORICAL_UNCONTRACTED:
            self.assertIn(repository_id, inspector.CHECK_CONTRACT)

    def test_pre_contract_dependency_evidence_admits_the_claim_end_to_end(self):
        merge_sha = "c8c1589db37ae7c5f0625994d23886c7dc1afb36"
        self.evidence["merge_sha"] = merge_sha
        self.evidence["ci"].update({"head_sha": merge_sha,
                                    "required_checks": sorted(inspector.BASELINE_RUST_CHECKS)})
        result = self.inspect_registry([self.claim])
        self.assertEqual(("clean", True), (result["classification"], result["admitted"]))
        self.evidence["merge_sha"] = "f" * 40
        self.evidence["ci"]["head_sha"] = "f" * 40
        result = self.inspect_registry([self.claim])
        self.assertFalse(result["admitted"])
        self.assertIn(
            "EVIDENCE_REQUIRED_CHECKS_NONNORMATIVE controller.evidence.evidence-dependency",
            result["errors"])

    def test_evidence_key_table_mirrors_the_controller_schema(self):
        self.assertEqual({"lineage"}, inspector.EVIDENCE_OPTIONAL_KEYS)
        self.assertEqual({"type", "generation", "supersedes"}, inspector.EVIDENCE_LINEAGE_KEYS)
        self.assertEqual({"aor_baseline"}, inspector.EVIDENCE_LINEAGE_TYPES)
        self.assertFalse(inspector.EVIDENCE_KEYS & inspector.EVIDENCE_OPTIONAL_KEYS)


if __name__ == "__main__": unittest.main()
