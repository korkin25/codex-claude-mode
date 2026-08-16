# Execution contract reference

## Repository identity

- Owner ID: `ccm-public`
- Canonical SSH remote: `git@github.com:korkin25/codex-claude-mode.git`
- Default branch: `main`
- Controller repository: `git@github.com:korkin25/ccm-multi.git`

Never persist local checkout paths: they are environment-specific. Never add a
Git HTTPS remote or consult a credential helper for repository access.
Run network Git with system/global configuration disabled, injected
`GIT_CONFIG_*`, `GIT_SSH*` and askpass variables removed, and
`GIT_SSH_COMMAND='ssh -o BatchMode=yes -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no'`.
The inspector performs the equivalent sanitization for its read-only Git calls.

## Inputs from the controller

Accept a task only when the root supplies all of these from one immutable
`ccm-multi` commit:

- claim-issuance controller commit SHA and a SHA-256 digest of canonical compact JSON for the
  exact claim object (`sort_keys=True`, separators `(',', ':')`, UTF-8);
- claim ID, generation, principal, exact issue/expiry times, branch, owner base
  SHA, exclusive capability set, and dependency evidence references;
- a canonical compact-JSON digest for every dependency evidence object;
- work item, public capability ID, acceptance criteria, required checks, and
  injected reporting route;
- the full base contract digest (exact manifest bytes, exact `TODO.md` bytes,
  normalized acceptance and complete capability declaration) and the external
  SHA-256 digest of the inspector taken from that same admitted base.

Use only the resolved sibling `ccm-multi` checkout and re-read the exact claim object at the separately measured current controller
SSH head when resuming. Require the issuance commit to remain its ancestor and
prefetched `origin/main` to equal that head. Require the current object to
remain identical, active, unexpired, and unsuperseded. Do
not substitute a mutable branch name, prose report, local checkout, PR label,
or CI result for exact object content.

Never start the mutable task-tree inspector directly. Use the trusted preflight
launcher obtained from an externally authenticated installation or extracted
and verified from the admitted base. It selects the inspector blob from the
exact externally supplied base SHA with bounded trusted Git, verifies the
external SHA-256 digest before interpreter startup, and runs the verified inode
through a private read-only descriptor in Python isolated mode. The inspector's
own base-blob and self-digest comparison is an additional defense, not the
bootstrap trust anchor.

## State invariants

The tracked state filename is derived from the validated claim ID. It is an
execution/recovery record, not the authoritative claim or capability registry.
It contains no authorization payload beyond identifiers and digests.

- One document represents one claim generation and one branch. Generation 1
  begins at the exact public base; later generations explicitly chain to the
  immediately preceding closed claim and pushed checkpoint SHA.
- The current controller registry contains one complete lineage for the same
  work item, owner repository, and exclusive capability set: exactly one claim
  exists for every generation through the current generation. A later claim
  names the unique generation `n-1` claim as its predecessor; sibling claims,
  generation gaps, forks, or capability-set aliases fail closed. Every claim
  in the current and issuance registries is validated against the central
  claim-schema semantics: exact keys and types, allowed status, ordered
  timezone-aware issue/expiry times, safe branch, non-empty unique capability
  IDs, generation, SHA, and evidence-reference formats. Every lineage
  predecessor is strictly terminal (`released`, `revoked`, or `expired`); an
  invented status is not closure.
- Externally measured SSH `main`, prefetched `origin/main`, and the admission
  base are equal. Later generations additionally require the named predecessor
  checkpoint to equal its separately measured latest remote head.
- `repository_id`, remote, work item, capability, base, branch, and generation
  are immutable within that document.
- Checkpoint sequences increase; every task commit updates the document and
  names its first parent in `checkpoint.parent_sha`.
- `completed_checks` and completed acceptance entries are append-only. Their
  prior order and bytes do not change; candidate state has no failed check.
- Acceptance is the exact normalized paragraph list under the manifest-bound
  `TODO.md` task heading, not agent prose. Its digest and the manifest's exact
  non-empty required-check list are immutable admission fields. Candidate
  completion equals that acceptance list and contains every required check.
- Every path changed across the full `base..HEAD` range is covered by the
  base-bound capability `content_scope`, apart from the exact state path for
  each fully validated lineage claim ID from generation 1 through `n`. The
  inspector recursively reads historical states at the exact checkpoint SHAs
  named by their successors and binds every admission, active-form claim
  digest, generation, and predecessor link. No `delivery/executions/` wildcard
  is allowed. Governance, skills, CI, `TODO.md`, and the manifest
  are therefore forbidden unless the immutable base declaration explicitly
  includes them.
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

Telegram destinations come only from active root/user instructions. The
pre-commit report is one or two substantive Russian lines describing the actual
diff and completed verification. It must not claim that the commit already
exists; a material diff change invalidates it and requires a refreshed report.

Post-push and post-merge reports are separate, detailed, factual Russian
messages. They include the change and its reason, affected components/material
files, security and authority impact (including an explicit `none` when
applicable), actual test outcomes, known limitations/blockers/unverified work,
exact branch and SHA, and clickable commit/PR/exact-SHA CI links. A merge report
also identifies source/target context and the exact merge SHA. Pending CI is
reported as pending and followed by its final result; it is never presented as
successful in advance.

No report contains secrets, credentials, private payloads, or routing, and no
plan or expected effect is described as completed. This public reference
intentionally contains no chat or topic IDs.
