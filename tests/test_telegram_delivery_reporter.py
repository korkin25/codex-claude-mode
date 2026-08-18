"""Drift gate and renderer tests for the vendored telegram-delivery-reporter skill.

The skill under `.agents/skills/telegram-delivery-reporter/` is a byte-for-byte
copy of the canonical implementation maintained in the private controller
repository. This module pins that copy to a recorded digest and exercises the
renderer paths that need neither network access nor a provider.
"""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SKILL_SCOPE = ".agents/skills/telegram-delivery-reporter/"
SKILL = ROOT / SKILL_SCOPE
VALIDATOR_SCRIPT = SKILL / "scripts/validate_report.py"

# Canonical source of this copy. The five skill files are taken verbatim from
# korkin25/ccm-multi at commit
# 47cf085e3d82e8cdb57af7bbc01e21a95ae3d861
# ("Merge pull request #46 from korkin25/fix/reporter-three-dot-changed-files").
# Re-synchronising the copy and updating EXPECTED_SKILL_DIGEST below must happen
# in one commit, so the recorded SHA always names the content that is checked in.
SOURCE_REPOSITORY = "korkin25/ccm-multi"
SOURCE_SHA = "47cf085e3d82e8cdb57af7bbc01e21a95ae3d861"
EXPECTED_SKILL_DIGEST = (
    "sha256:e0df1b81b8f5877622eacf82dca05c403535a703b13645b979cd1bda63f2247e"
)
SKILL_FILES = (
    "SKILL.md",
    "agents/openai.yaml",
    "references/report-contract.md",
    "scripts/validate_report.py",
    "scripts/verify_report.py",
)
RESYNC_INSTRUCTIONS = (
    "The vendored telegram-delivery-reporter copy no longer matches the recorded "
    f"digest. Re-copy every file of {SKILL_SCOPE} from {SOURCE_REPOSITORY} at the "
    f"recorded source commit {SOURCE_SHA} (or at the newer canonical commit you "
    "are adopting), then update SOURCE_SHA and EXPECTED_SKILL_DIGEST in this file "
    "IN THE SAME COMMIT, so the recorded SHA never names content other than the "
    "content checked in. Never hand-edit the copy: the canonical implementation "
    "lives in the controller repository and every change belongs there first."
)


def load_module(name, path):
    """Import a skill script by path without relying on `assert` (`python -O`)."""
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    # verify_report.py does `import validate_report`, so the renderer must be
    # reachable under its own module name before either module is executed.
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


reporter = load_module("validate_report", VALIDATOR_SCRIPT)
verifier = load_module("verify_report", SKILL / "scripts/verify_report.py")
capabilities = load_module(
    "validate_capabilities", ROOT / "scripts" / "validate_capabilities.py"
)

REPOSITORY = "korkin25/codex-claude-mode"
CANDIDATE = "b" * 40
MERGE = "c" * 40


def git(*args, text=True):
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, text=text,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ).stdout


def tracked_skill_files():
    """Return (mode, repository-relative path) for every tracked file of the skill.

    Only tracked files count. Byte-compiled caches and other build residue that
    Python may drop next to the scripts are not part of the vendored copy and
    must not move its digest.
    """
    entries = []
    for entry in git("ls-files", "-s", "-z", "--", SKILL_SCOPE, text=False).split(b"\0"):
        if not entry:
            continue
        meta, path = entry.split(b"\t", 1)
        mode, _blob, _stage = meta.split(b" ")
        entries.append((mode.decode("ascii"), path.decode("utf-8")))
    if not entries:
        raise RuntimeError(f"{SKILL_SCOPE} has no tracked files; stage the copy first")
    return sorted(entries)


def canonical_skill_digest():
    """Digest the skill with the repository's canonical content-scope formula.

    `scripts/validate_capabilities.py::canonical_tree_digest` hashes a content
    scope as the deduplicated, sorted, NUL-terminated `git ls-tree -rz
    --full-tree` records of that scope. The same records are rebuilt here — mode,
    type, blob ID and repository-relative path — but the blob ID comes from the
    bytes on disk, so an edit is caught before it is committed as well as after.
    """
    records = set()
    for mode, path in tracked_skill_files():
        payload = (ROOT / path).read_bytes()
        blob = hashlib.sha1(b"blob %d\0" % len(payload) + payload).hexdigest()
        records.add(f"{mode} blob {blob}\t{path}".encode("utf-8"))
    canonical = b"".join(record + b"\0" for record in sorted(records))
    return "sha256:" + hashlib.sha256(canonical).hexdigest()


class VendoredSkillDigestTests(unittest.TestCase):
    def test_vendored_skill_matches_the_recorded_source_digest(self):
        self.assertEqual(
            EXPECTED_SKILL_DIGEST, canonical_skill_digest(), RESYNC_INSTRUCTIONS
        )

    def test_vendored_skill_carries_exactly_the_recorded_file_set(self):
        present = [path[len(SKILL_SCOPE):] for _mode, path in tracked_skill_files()]
        self.assertEqual(sorted(SKILL_FILES), present, RESYNC_INSTRUCTIONS)
        for _mode, path in tracked_skill_files():
            with self.subTest(file=path):
                self.assertTrue((ROOT / path).is_file())

    def test_digest_formula_is_the_repository_content_scope_formula(self):
        """Prove the local formula is the one `validate_capabilities.py` uses."""
        dirty = git("status", "--porcelain", "--untracked-files=no", "--", SKILL_SCOPE)
        if dirty.strip():
            self.skipTest("skill copy differs from HEAD; nothing to cross-check")
        errors = []
        measured = capabilities.canonical_tree_digest(
            ROOT, "HEAD", [SKILL_SCOPE], "telegram-delivery-reporter", errors
        )
        self.assertEqual([], errors)
        self.assertEqual(measured, canonical_skill_digest())

    def test_vendored_skill_stays_free_of_route_and_topic_identity(self):
        for name in SKILL_FILES:
            text = (SKILL / name).read_text(encoding="utf-8")
            with self.subTest(file=name):
                self.assertIsNone(re.search(r"-100\d{10}", text))
                self.assertIsNone(re.search(r"\bbot\d{6,}:", text))
        for event, keys in reporter.EVENT_KEYS.items():
            with self.subTest(event=event):
                self.assertEqual(
                    set(), keys & {"chat_id", "message_thread_id", "token", "route"}
                )

    def test_skill_layout_resolves_the_verifier_repository_root(self):
        self.assertEqual(ROOT, verifier.SKILL_REPOSITORY_ROOT)


class ReportRenderingTests(unittest.TestCase):
    def postmerge(self):
        return {
            "event": "postmerge", "repository": REPOSITORY,
            "change": "перенесён merge-only reporting skill",
            "reason": "отправлять только доказанные успешные merge",
            "components": ["SKILL.md", "validate_report.py"],
            "security_authority_impact": "route authority не изменена; секреты исключены",
            "tests": ["Public capability manifest: success"],
            "limitations": "exactly-once зависит от сохранённого receipt",
            "blockers": "нет", "unverified": "нет",
            "source_branch": "feature/telegram-delivery-reporter", "target_branch": "main",
            "candidate_sha": CANDIDATE, "merge_sha": MERGE,
            "commit_url": f"https://github.com/{REPOSITORY}/commit/{MERGE}",
            "pr_url": f"https://github.com/{REPOSITORY}/pull/11",
            "ci": {"state": "success", "head_sha": MERGE,
                   "url": f"https://github.com/{REPOSITORY}/actions/runs/123"},
            "files": [f"{SKILL_SCOPE}SKILL.md"],
        }

    def render(self, payload):
        capability = reporter._verified_report(
            copy.deepcopy(payload), ["fixture-provider-check"]
        )
        return reporter.render(capability)

    def headlines(self, rendered):
        return [line for line in rendered.split("\n") if "<b>" in line]

    def test_direct_script_invocation_rejects_raw_event_json(self):
        completed = subprocess.run(
            [sys.executable, str(VALIDATOR_SCRIPT)], cwd=ROOT, text=True,
            input=json.dumps(self.postmerge()),
            env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        self.assertEqual(1, completed.returncode)
        self.assertEqual("", completed.stdout)
        self.assertEqual(
            {"valid": False,
             "errors": ["direct rendering forbidden; use verify_report.py"]},
            json.loads(completed.stderr),
        )

    def test_in_process_renderer_refuses_an_unverified_payload(self):
        with self.assertRaisesRegex(reporter.ReportError, "verification required"):
            reporter.render(self.postmerge())

    def test_canonical_layout_keeps_section_order_and_route_without_shas(self):
        rendered = self.render(self.postmerge())
        self.assertEqual([
            f"✅ <b>{REPOSITORY}: merge завершён</b>",
            "🎯 <b>Зачем</b>",
            "🧩 <b>Что изменилось</b>",
            "📄 <b>Изменённые файлы</b>",
            "🧪 <b>Проверено</b>",
            "🔐 <b>Безопасность и полномочия</b>",
            "⚠️ <b>Ограничения</b>",
            "🔗 <b>Ссылки</b>",
        ], self.headlines(rendered))
        lines = rendered.split("\n")
        self.assertEqual("перенесён merge-only reporting skill", lines[1])
        self.assertEqual("", lines[2])
        self.assertIn(
            "• PR #11: merged, <code>feature/telegram-delivery-reporter</code> "
            "→ <code>main</code>",
            lines,
        )
        self.assertNotIn(CANDIDATE, rendered)
        self.assertIn(f"• <code>{SKILL_SCOPE}SKILL.md</code>", lines)

    def test_limitations_block_disappears_when_every_caveat_is_checked_empty(self):
        for markers in (("нет", "нет", "нет"), ("нет известных", "Нет", "отсутствуют.")):
            with self.subTest(markers=markers):
                payload = self.postmerge()
                payload["limitations"], payload["blockers"], payload["unverified"] = markers
                rendered = self.render(payload)
                self.assertNotIn("⚠️", rendered)
                self.assertNotIn("Ограничения", rendered)
                self.assertNotIn("Блокеры", rendered)
                self.assertNotIn("Непроверенное", rendered)
                self.assertNotIn("\n\n\n", rendered)
                self.assertEqual(
                    ["🔐 <b>Безопасность и полномочия</b>", "🔗 <b>Ссылки</b>"],
                    self.headlines(rendered)[-2:],
                )
        payload = self.postmerge()
        payload["limitations"] = "нет"
        payload["blockers"] = "публикация ждёт ручного подтверждения"
        rendered = self.render(payload)
        self.assertIn("⚠️ <b>Ограничения</b>", rendered)
        self.assertEqual(
            ["• Блокеры: публикация ждёт ручного подтверждения"],
            [line for line in rendered.split("\n")
             if line.startswith("• ") and re.match(
                 r"• (Ограничения|Блокеры|Непроверенное):", line)],
        )

    def test_merge_commit_link_is_short_while_href_keeps_the_full_sha(self):
        rendered = self.render(self.postmerge())
        self.assertEqual(8, reporter.SHORT_SHA_CHARS)
        self.assertIn(f'/commit/{MERGE}">commit {MERGE[:8]}</a>', rendered)
        visible = re.sub(r'href="[^"]+"', 'href=""', rendered)
        self.assertNotIn(MERGE, visible)
        self.assertNotIn("https://", visible)
        self.assertEqual(1, visible.count(f"commit {MERGE[:8]}"))
        self.assertEqual(3, rendered.rsplit("\n", 1)[-1].count(" · "))

    def test_dynamic_text_is_escaped_and_only_b_code_a_tags_are_emitted(self):
        payload = self.postmerge()
        payload.update({
            "change": 'change <b>x</b> & "quoted"',
            "components": ["component <script>"],
            "files": ['src/<tag>&".py'],
        })
        rendered = self.render(payload)
        self.assertIn("change &lt;b&gt;x&lt;/b&gt; &amp; &quot;quoted&quot;", rendered)
        self.assertIn("• component &lt;script&gt;", rendered)
        self.assertIn("<code>src/&lt;tag&gt;&amp;&quot;.py</code>", rendered)
        self.assertNotIn("<script>", rendered)
        tags = re.findall(r"</?([A-Za-z0-9]+)(?: [^>]*)?>", rendered)
        self.assertTrue(tags)
        self.assertLessEqual(set(tags), {"a", "b", "code"})

    def test_message_length_is_bounded_in_utf16_code_units(self):
        self.assertEqual(4096, reporter.MAX_MESSAGE_CHARS)
        self.assertEqual(2, reporter.telegram_length("🧩"))
        self.assertEqual(4096, reporter.telegram_length("🧩" * 2048))
        capability = reporter._verified_report(
            self.postmerge(), ["fixture-provider-check"]
        )
        for filler in ("x" * 4096, "🧩" * 2048):
            with self.subTest(units=4096):
                with mock.patch.object(
                    reporter, "_render_payload", return_value=filler
                ):
                    self.assertEqual(filler, reporter.render(capability))
        for filler in ("x" * 4097, "🧩" * 2049):
            with self.subTest(units=4098):
                with mock.patch.object(
                    reporter, "_render_payload", return_value=filler
                ):
                    with self.assertRaisesRegex(reporter.ReportError, "exceeds 4096"):
                        reporter.render(capability)

    def test_route_fields_and_unsupported_events_fail_closed(self):
        payload = self.postmerge()
        payload["chat_id"] = 1
        with self.assertRaisesRegex(reporter.ReportError, "unknown"):
            self.render(payload)
        for event in ("precommit", "postpush", "ci_terminal"):
            with self.subTest(event=event):
                payload = self.postmerge()
                payload["event"] = event
                with self.assertRaisesRegex(reporter.ReportError, "unsupported"):
                    self.render(payload)
        for state in ("pending", "in_progress", "failure", "cancelled"):
            with self.subTest(state=state):
                payload = self.postmerge()
                payload["ci"]["state"] = state
                with self.assertRaisesRegex(reporter.ReportError, "successful"):
                    self.render(payload)



class ChangedPathFixtureTests(unittest.TestCase):
    """Real-Git regression for a PR base head that moved after the PR opened.

    The vendored copy shipped before this sync measured the changed files with a
    two-dot `git diff base candidate`, while the provider publishes the PR file
    set as the three-dot comparison from the fork point. These fixtures build the
    "base moved" history in a throwaway repository, so the regression is proven
    locally without a provider, a token, or the network.
    """

    def git(self, root, *args):
        executable = verifier._trusted_executable(verifier.GIT_CANDIDATES, "git")
        identity = {
            "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "HOME": str(root),
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_AUTHOR_NAME": "Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
            "GIT_COMMITTER_NAME": "Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
            "GIT_AUTHOR_DATE": "2026-01-01T00:00:00+0000",
            "GIT_COMMITTER_DATE": "2026-01-01T00:00:00+0000",
        }
        completed = subprocess.run(
            [str(executable), *args], cwd=root, env=identity, check=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60,
        )
        return completed.stdout.decode("utf-8").strip()

    def commit(self, root, name, message):
        (root / name).write_text(f"{name}\n", encoding="utf-8")
        self.git(root, "add", "--", name)
        self.git(root, "commit", "--no-gpg-sign", "-m", message)
        return self.git(root, "rev-parse", "HEAD")

    def moved_base_repository(self, root):
        """Build base, a branch forked from it, then a later foreign base commit."""
        self.git(root, "init", "--initial-branch=main", "--quiet")
        fork_point = self.commit(root, "shared.txt", "shared base")
        self.git(root, "checkout", "--quiet", "-b", "feature")
        candidate = self.commit(root, "candidate.txt", "candidate work")
        self.git(root, "checkout", "--quiet", "main")
        moved_base = self.commit(root, "foreign.txt", "unrelated PR merged into main")
        return fork_point, candidate, moved_base

    def test_moved_pr_base_measures_the_three_dot_path_set(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(strict=True)
            fork_point, candidate, moved_base = self.moved_base_repository(root)
            self.assertNotEqual(fork_point, moved_base)
            provider = verifier.Provider()

            measured_fork, paths = provider.changed_paths(root, moved_base, candidate)
            self.assertEqual(fork_point, measured_fork)
            self.assertEqual(["candidate.txt"], paths)
            self.assertEqual(fork_point, provider.merge_base(root, moved_base, candidate))

            # The provider (GitHub) reports exactly the three-dot set; the
            # two-dot diff the verifier used before additionally reports the
            # foreign commit's files, inverted, and would fail the file match.
            two_dot = provider._git(
                root, "diff", "--name-only", "--diff-filter=ACDMRTUXB", "-z",
                moved_base, candidate, "--", raw=True,
            )
            self.assertEqual(
                {"candidate.txt", "foreign.txt"}, set(two_dot.rstrip("\0").split("\0"))
            )
            three_dot = provider._git(
                root, "diff", "--name-only", "--diff-filter=ACDMRTUXB", "-z",
                f"{moved_base}...{candidate}", "--", raw=True,
            )
            self.assertEqual(["candidate.txt"], three_dot.rstrip("\0").split("\0"))

    def test_absent_local_merge_base_fails_closed_with_an_explicit_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(strict=True)
            self.git(root, "init", "--initial-branch=main", "--quiet")
            base = self.commit(root, "shared.txt", "shared base")
            self.git(root, "checkout", "--quiet", "--orphan", "unrelated")
            self.git(root, "rm", "--quiet", "-rf", ".")
            unrelated = self.commit(root, "unrelated.txt", "unrelated history")
            provider = verifier.Provider()
            with self.assertRaisesRegex(
                verifier.VerificationError, "no locally reachable merge base"
            ):
                provider.changed_paths(root, base, unrelated)


if __name__ == "__main__":
    unittest.main()
