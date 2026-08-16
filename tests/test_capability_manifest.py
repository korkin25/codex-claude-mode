import copy
import importlib.util
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "validate_capabilities", ROOT / "scripts" / "validate_capabilities.py"
)
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


class CapabilityManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        shutil.copytree(ROOT / "delivery", self.root / "delivery")
        shutil.copy(ROOT / "TODO.md", self.root / "TODO.md")

    def tearDown(self):
        self.temp.cleanup()

    def manifest(self):
        return json.loads((self.root / "delivery" / "capabilities.json").read_text())

    def write_manifest(self, manifest):
        (self.root / "delivery" / "capabilities.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )

    def test_checked_in_manifest_is_valid(self):
        self.assertEqual(validator.validate_repository(self.root), [])

    def test_planned_capability_cannot_publish_verification(self):
        manifest = self.manifest()
        manifest["capabilities"][1]["verification"] = {
            "merge_sha": "a" * 40,
            "content_digest": "sha256:" + "b" * 64,
            "ci": {
                "provider": "github-actions",
                "run_id": "123",
                "url": validator.REPOSITORY + "/actions/runs/123",
                "head_sha": "a" * 40,
                "status": "success",
                "required_checks": manifest["capabilities"][1]["required_checks"],
            },
            "verified_at": "2026-08-17T00:00:00Z",
            "provenance": "measured",
        }
        self.write_manifest(manifest)
        self.assertTrue(any("PREMATURE_EVIDENCE" in error for error in validator.validate_repository(self.root)))

    def test_verified_capability_requires_exact_ci_sha(self):
        manifest = self.manifest()
        capability = manifest["capabilities"][0]
        capability["status"] = "verified"
        capability["verification"] = {
            "merge_sha": "a" * 40,
            "content_digest": "sha256:" + "b" * 64,
            "ci": {
                "provider": "github-actions",
                "run_id": "123",
                "url": validator.REPOSITORY + "/actions/runs/123",
                "head_sha": "c" * 40,
                "status": "success",
                "required_checks": capability["required_checks"],
            },
            "verified_at": "2026-08-17T00:00:00Z",
            "provenance": "measured",
        }
        self.write_manifest(manifest)
        errors = validator.validate_repository(self.root)
        self.assertTrue(any("SHA_MISMATCH" in error for error in errors))

    def test_unknown_dependency_fails(self):
        manifest = self.manifest()
        manifest["capabilities"][1]["dependencies"] = ["ccm.missing.v1"]
        self.write_manifest(manifest)
        self.assertTrue(any("UNKNOWN_DEPENDENCY" in error for error in validator.validate_repository(self.root)))

    def test_dependency_cycle_fails(self):
        manifest = self.manifest()
        manifest["capabilities"][0]["status"] = "planned"
        manifest["capabilities"][0]["dependencies"] = ["ccm.ctl.v1"]
        manifest["capabilities"][2]["dependencies"] = ["ccm.serve.v1"]
        self.write_manifest(manifest)
        self.assertTrue(any("DEPENDENCY_CYCLE" in error for error in validator.validate_repository(self.root)))

    def test_todo_status_drift_fails(self):
        todo = (self.root / "TODO.md").read_text(encoding="utf-8")
        todo = todo.replace("| `CCM-SERVE-001` | `ready` |", "| `CCM-SERVE-001` | `planned` |")
        (self.root / "TODO.md").write_text(todo, encoding="utf-8")
        self.assertTrue(any("TODO_STATUS_MISMATCH" in error for error in validator.validate_repository(self.root)))

    def test_todo_dependency_drift_fails(self):
        todo = (self.root / "TODO.md").read_text(encoding="utf-8")
        todo = todo.replace(
            "| `CCM-SERVE-001` |\n| `CCM-SKILL-001`",
            "| — |\n| `CCM-SKILL-001`",
            1,
        )
        (self.root / "TODO.md").write_text(todo, encoding="utf-8")
        self.assertTrue(any("TODO_DEPENDENCY_MISMATCH" in error for error in validator.validate_repository(self.root)))

    def test_blocked_capability_requires_public_reason(self):
        manifest = self.manifest()
        manifest["capabilities"][0]["status"] = "blocked"
        self.write_manifest(manifest)
        self.assertTrue(any("BLOCKER_MISSING" in error for error in validator.validate_repository(self.root)))

    def test_schema_and_validator_model_cannot_drift(self):
        path = self.root / "delivery" / "capabilities.schema.json"
        schema = json.loads(path.read_text(encoding="utf-8"))
        schema["$defs"]["capability"]["required"].remove("required_checks")
        path.write_text(json.dumps(schema, indent=2) + "\n", encoding="utf-8")
        self.assertTrue(any("SCHEMA_REQUIRED_DRIFT" in error for error in validator.validate_repository(self.root)))

    def test_verified_evidence_happy_path(self):
        manifest = self.manifest()
        capability = manifest["capabilities"][0]
        capability["content_scope"] = ["TODO.md"]
        self.write_manifest(manifest)
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        subprocess.run(["git", "config", "user.name", "Capability Test"], cwd=self.root, check=True)
        subprocess.run(["git", "config", "user.email", "capability@example.invalid"], cwd=self.root, check=True)
        subprocess.run(["git", "add", "."], cwd=self.root, check=True)
        subprocess.run(["git", "commit", "-qm", "implementation"], cwd=self.root, check=True)
        merge_sha = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=self.root, check=True,
            capture_output=True, text=True,
        ).stdout.strip()
        digest_errors = []
        digest = validator.canonical_tree_digest(
            self.root, merge_sha, capability["content_scope"], "test", digest_errors
        )
        self.assertEqual(digest_errors, [])
        self.assertIsNotNone(digest)
        capability["status"] = "verified"
        capability["verification"] = {
            "merge_sha": merge_sha,
            "content_digest": digest,
            "ci": {
                "provider": "github-actions",
                "run_id": "123",
                "url": validator.REPOSITORY + "/actions/runs/123",
                "head_sha": merge_sha,
                "status": "success",
                "required_checks": capability["required_checks"],
            },
            "verified_at": "2026-08-17T00:00:00Z",
            "provenance": "measured",
        }
        self.write_manifest(manifest)
        todo = (self.root / "TODO.md").read_text(encoding="utf-8")
        todo = todo.replace("| `CCM-SERVE-001` | `ready` |", "| `CCM-SERVE-001` | `done` |")
        (self.root / "TODO.md").write_text(todo, encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=self.root, check=True)
        subprocess.run(["git", "commit", "-qm", "settle evidence"], cwd=self.root, check=True)
        self.assertEqual(validator.validate_repository(self.root), [])
        capability["content_scope"] = ["delivery/"]
        self.write_manifest(manifest)
        self.assertTrue(any(
            "DECLARATION_CHANGED_DURING_SETTLEMENT" in error
            for error in validator.validate_repository(self.root)
        ))
        capability["content_scope"] = ["TODO.md"]
        capability["specification"] = "TODO.md#ccm-direct-001"
        self.write_manifest(manifest)
        self.assertTrue(any(
            "DECLARATION_CHANGED_DURING_SETTLEMENT ccm.serve.v1.specification" in error
            for error in validator.validate_repository(self.root)
        ))
        capability["specification"] = "TODO.md#ccm-serve-001"
        added = copy.deepcopy(capability)
        added["id"] = "ccm.new.v1"
        added["work_item"] = "CCM-NEW-001"
        added["specification"] = "TODO.md#ccm-new-001"
        manifest["capabilities"].append(added)
        self.write_manifest(manifest)
        todo = (self.root / "TODO.md").read_text(encoding="utf-8")
        todo += "\n### CCM-NEW-001\n\nTest-only capability.\n"
        todo = todo.replace(
            "| `CCM-PROMPT-001` | `done` | `maintenance` |",
            "| `CCM-NEW-001` | `done` | `core` | Test-only capability | — |\n"
            "| `CCM-PROMPT-001` | `done` | `maintenance` |",
        )
        (self.root / "TODO.md").write_text(todo, encoding="utf-8")
        self.assertTrue(any(
            "VERIFIED_WITHOUT_BASELINE ccm.new.v1" in error
            for error in validator.validate_repository(self.root)
        ))

    def test_specification_anchor_must_exist(self):
        manifest = self.manifest()
        manifest["capabilities"][0]["specification"] = "TODO.md#missing-anchor"
        self.write_manifest(manifest)
        self.assertTrue(any("SPECIFICATION_NOT_FOUND" in error for error in validator.validate_repository(self.root)))

    def test_duplicate_json_key_fails(self):
        path = self.root / "delivery" / "capabilities.json"
        path.write_text(
            '{"document_type":"ccm-capability-manifest","document_type":"duplicate"}',
            encoding="utf-8",
        )
        self.assertTrue(any("duplicate JSON key" in error for error in validator.validate_repository(self.root)))


if __name__ == "__main__":
    unittest.main()
