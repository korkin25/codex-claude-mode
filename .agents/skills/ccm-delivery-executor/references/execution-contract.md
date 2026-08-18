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
- work item, public capability ID, acceptance criteria, and required checks;
- the full base contract digest (exact manifest bytes, exact `TODO.md` bytes,
  normalized acceptance and complete capability declaration) and the external
  SHA-256 digest of the inspector taken from that same admitted base.

Use only the resolved sibling `ccm-multi` checkout and re-read the exact claim object at the separately measured current controller
SSH head when resuming. Require the issuance commit to remain its ancestor and
prefetched `origin/main` to equal that head. Require the current object's
admission-bound identity fields to remain identical, and the claim to remain
active, unexpired, and unsuperseded. `admission.claim_digest` is the canonical
digest of exactly those identity fields plus `status: active`, so bounded-lane
scope fields the controller may add never silently rebind an admission. Every
other active claim in the registry must stay disjoint from this one under the
controller's own conflict rules: a different owner repository, work item, and
exclusive capability, no overlapping `write_paths` inside one repository, and
no shared `content_scope`. A claim carrying no `write_paths` claims its whole
repository, which the repository rule already rejects. `content_scopes` gets no
such fallback because it is the only rule that reaches across repositories, so
a claim on either side of the comparison that declares no well-formed
`content_scopes` conflicts with every concurrent lane. The registry may hold at
most three active claims, and no claim may hold a lease longer than twelve
hours. Do
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

The launcher obtains the authoritative private checkout from the fixed
owner-only configuration
`~/.config/codex-claude-mode/delivery-executor.json`, resolved from the OS
account rather than `HOME` or `XDG_CONFIG_HOME`. The file is a canonical,
non-symlink regular file owned by the effective user with mode `0600` and exact
schema `{"schema_version":1,"canonical_root":"/absolute/path"}`. Duplicate
keys fail closed and `schema_version` must have exact JSON integer
type and value `1`; JSON booleans are not accepted as integer aliases. A supplied
`--task-root` is only checked for exact lexical and resolved equality with that
configured root. The downstream argument list cannot nominate `--root` in
exact, attached, repeated, or abbreviated form; preflight injects its one
authenticated canonical root after rejecting all such forms.

## State invariants

The tracked state filename is derived from the validated claim ID. It is an
execution/recovery record, not the authoritative claim or capability registry.
It contains no authorization payload beyond identifiers and digests.
The tracked execution state and every claims, delivery-state, and evidence
controller snapshot require `schema_version` with exact JSON integer type and
value `1`; booleans are rejected rather than treated as integer aliases.

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
  IDs, generation, SHA, and evidence-reference formats. A claim may additionally
  carry the bounded-lane scope fields `write_paths` and `content_scopes`; they
  are optional because the append-only registry keeps records issued before
  scoping existed, and when present they are validated with the same strictness
  (non-empty, unique, lexically ordered canonical paths whose only wildcard is a
  trailing `/**`, and identifier-shaped scopes). Any other key still fails
  closed. The inspector loads and
  strictly validates `claims.json`, `state.json`, and `evidence.json` both at
  the current controller head and at each lineage state's exact
  `admission.controller_commit_sha`. Repository, work-item, capability,
  dependency, external-prerequisite, and evidence references must all resolve
  within that same snapshot; claim capabilities are an owner-matching subset
  of the work item's capabilities. At issuance the work item is already
  `ready`, every dependency is `done`, and every external prerequisite is
  `available`. Dependency evidence is `merge_ci` evidence from the dependency
  work item's owner repository, and its `required_checks` are that repository's
  complete normative check contract. A GitHub check name comes from the
  workflow in the merged tree, so a merge cannot have run a job added later:
  an enumerated set of pre-contract merges, pinned by immutable repository and
  merge SHA, instead carries the complete contract measured at that exact
  commit, and the one of them whose tree predates the owner capability manifest
  carries no `check_contract`. That exemption admits a null `check_contract`
  and nothing else: a `ccm-public` merge SHA must still name a commit reachable
  from this repository's HEAD whether or not the record binds a manifest, and
  only the manifest digest itself is unmeasurable for a tree that predates the
  file. Both tables list only merges some admission path actually consumes as
  dependency evidence, because normative validation runs nowhere else. They are
  exhaustive over the past, not closed forever: the next time a repository's
  check contract grows, records issued under today's contract need a new entry
  here, which is a public change. Every other record, and therefore every new
  merge, must match the current contract exactly. An external capability's exact
  `evidence_refs` are the only normative representation of a cross-owner
  evidence relation; owner relationships are never inferred from an ID or
  prose. Evidence `verified_at` is no later than claim `issued_at`, and the
  claim/state issue, expiry, and checkpoint ordering remains valid. Historical
  evidence objects and their canonical digests are checked at their issuance
  snapshot, never filled from a later registry. An evidence record may
  additionally carry the AOR baseline `lineage`; it is optional because the
  append-only registry keeps baseline records written before the lineage
  existed, and when present it is exactly `type`, `generation`, and
  `supersedes`: an `aor_baseline` type on an `aor`-owned
  `evidence-aor-baseline-*` record, an integer generation of at least one, and
  either `null` at generation one or the evidence ID of the record it replaces.
  Any other key inside it still fails closed. Within one snapshot the chain is
  append-only: one record per generation, contiguous from one, each naming the
  immediately preceding generation, so forks, gaps, dangling supersessions, and
  cycles fail closed. A chain that survives all of that has a tip, and once a
  snapshot carries one, the single evidence reference of the
  `external.aor-baseline-observed` capability must be exactly that tip: a
  superseded baseline is not an observation of the current one. A snapshot older
  than the lineage carries no chain and no such requirement. Every lineage
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
  named by their successors, proves every recorded controller commit is
  available and ancestral to the externally measured controller head, and
  binds every admission, active-form claim/evidence digest, generation, and
  predecessor link. No `delivery/executions/` wildcard
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

The executor never sends Telegram messages before commit, after push, for
candidate CI, or after merge. Telegram routing is not an executor input, and a
missing route never blocks scoped edits, tests, state checkpoints, commits, SSH
pushes, or candidate handoff.

The root controller alone sends exactly one factual merge report after the
hosting provider has confirmed the branch merge and every required post-merge
CI check has completed successfully on the exact merge SHA. The report binds
the change and reason, affected components/material files, security and
authority impact, actual review and test outcomes, known limitations/blockers/
unverified work, source/target context, reviewed candidate SHA, exact merge SHA,
and clickable merge commit, PR, and exact merge-SHA CI links.

Pending, failed, cancelled, or unverified post-merge CI produces no Telegram
message. A correction is allowed only for an already sent merge report. No
report contains secrets, credentials, private payloads, or routing, and no plan
or expected effect is described as completed. This public reference
intentionally contains no chat or topic IDs.
