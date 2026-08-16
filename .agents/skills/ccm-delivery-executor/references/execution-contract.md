# Execution contract reference

## Repository identity

- Owner ID: `ccm-public`
- Canonical SSH remote: `git@github.com:korkin25/codex-claude-mode.git`
- Default branch: `main`
- Controller repository: `git@github.com:korkin25/ccm-multi.git`

Never persist local checkout paths: they are environment-specific. Never add a
Git HTTPS remote or consult a credential helper for repository access.

## Inputs from the controller

Accept a task only when the root supplies all of these from one immutable
`ccm-multi` commit:

- controller commit SHA and a SHA-256 digest of canonical compact JSON for the
  exact claim object (`sort_keys=True`, separators `(',', ':')`, UTF-8);
- claim ID, generation, principal, exact issue/expiry times, branch, owner base
  SHA, exclusive capability set, and dependency evidence references;
- work item, public capability ID, acceptance criteria, required checks, and
  injected reporting route.

Re-read the exact claim object when resuming. Do not substitute a newer mutable
branch, prose report, local checkout, PR label, or CI result for it.

## State invariants

The tracked state filename is derived from the validated claim ID. It is an
execution/recovery record, not the authoritative claim or capability registry.
It contains no authorization payload beyond identifiers and digests.

- One document represents one claim generation and one branch.
- `repository_id`, remote, work item, capability, base, branch, and generation
  are immutable within that document.
- Checkpoint sequences increase; every task commit updates the document and
  names its first parent in `checkpoint.parent_sha`.
- `completed_checks` contains only commands already run with their outcomes.
- `remaining_work` and `next_action` describe unfinished work; neither is an
  assertion that the work happened.
- `candidate` is a phase, not evidence. The exact candidate is the clean local
  HEAD only after SSH push equality is measured.

The state may remain in merged history as a public audit/recovery artifact. It
must not contain private routing, secrets, raw prompts, approval bodies, or
private product documents.

## Authority gates

The claim authorizes scoped candidate preparation. Root retains PR, review,
merge, evidence settlement, and cross-repository coordination. Direct user
authority remains mandatory for architecture/contract choices, new runtime
dependencies, releases, deployments, secrets, destructive or paid actions,
and genuine ambiguity.

Telegram destinations come only from active root/user instructions. Reports
must be in Russian, factual, free of secrets, and include a clickable commit
link after push. This public reference intentionally contains no chat or topic
IDs.
