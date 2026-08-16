---
name: ccm-delivery-executor
description: Execute or recover exactly one claimed codex-claude-mode delivery task under the ccm-multi controller contract. Use when Codex receives an exact ccm-public claim, owner base SHA, capability, branch, and dependency evidence; must preserve restart-safe branch state; or must diagnose whether an interrupted public-repository task is clean, dirty, stale, diverged, or blocked by an expired claim.
---

# CCM Delivery Executor

Execute one admitted public-client task. Treat the `ccm-multi` claim as scoped
input, not standing authority or permission to merge.

## Establish the boundary

1. Read root/user instructions, repository `AGENTS.md`, `TODO.md`, `ROADMAP.md`,
   `ARCHITECTURE.md`, `SECURITY.md`, and `delivery/capabilities.json`.
2. Read [references/execution-contract.md](references/execution-contract.md).
3. Require one complete claim obtained from an exact `ccm-multi` commit: claim
   ID and generation, principal, `repository_id=ccm-public`, exact base SHA,
   task branch, exclusive capability set, issue/expiry times, and dependency
   evidence references and canonical digests of every referenced evidence
   object. Require the claimed task and capability to agree with the public
   manifest and task registry. On recovery, measure the current `ccm-multi`
   SSH `main` head and validate the claim there again; its issuance snapshot
   cannot hide later revocation or supersession.
4. Fetch only through the canonical SSH remote. Require a clean isolated
   worktree/clone whose `main` and remote `main` equal the claim base. Never use
   Git HTTPS or a credential store.
5. Stop on missing, ambiguous, stale, expired, mismatched, or conflicting
   inputs. A claim permits preparation of a candidate only. It cannot grant
   architecture, release, deployment, secret, destructive, cost-bearing, or
   merge authority.

Do not implement `CCM-SERVE-001` or any other task unless that exact task is
claimed and explicitly dispatched. Do not use `codex2meet`.

## Create durable execution state

Use one tracked state document per claim at
`delivery/executions/<claim-id>.json`. Validate it with the stdlib tool and the
descriptive schema in [references/execution-state.schema.json](references/execution-state.schema.json):

```bash
PYTHONDONTWRITEBYTECODE=1 python3 \
  .agents/skills/ccm-delivery-executor/scripts/inspect_state.py validate \
  --state delivery/executions/<claim-id>.json
```

The first task-branch commit must contain the `claimed` state and bind the
exact controller commit, claim/evidence digests, public base, branch,
capabilities, and claim generation. Generation 1 starts directly at the claim
base (`HEAD^ == base_sha`). A later generation explicitly names the immediately
preceding closed claim and its exact pushed checkpoint SHA. Every later
task-branch commit must update that same state:
increment `checkpoint.sequence`, set `checkpoint.parent_sha` to the commit's
first parent, update phase/kind/time, record facts already checked, and state a
concrete `next_action`. Timestamps and completed checks/acceptance are
monotonic and append-only. Never put secrets, prompts, credentials, raw approval
payloads, or private Telegram routing in it.

Keep the state and implementation in the same commit. After commit, require the
state's `checkpoint.parent_sha` to equal `HEAD^`; after push, require local HEAD
to equal the measured SSH branch head. This makes the pushed branch, rather
than terminal memory, the restart checkpoint.

## Recover before writing

On every fresh process or after an abrupt/planned restart, do no implementation
writes until this command returns `classification: clean` and `admitted: true`:

```bash
env -u GIT_CONFIG_COUNT -u GIT_CONFIG_KEY_0 -u GIT_CONFIG_VALUE_0 \
  GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null \
  PYTHONDONTWRITEBYTECODE=1 python3 \
  .agents/skills/ccm-delivery-executor/scripts/inspect_state.py inspect \
  --root . --state delivery/executions/<claim-id>.json \
  --at <timezone-aware-RFC3339> --remote-head <40-hex-SSH-branch-head> \
  --controller-root <isolated-ccm-multi-checkout> \
  --controller-head <40-hex-SSH-main-head>
```

The inspector does not access the network. Fetch and measure both heads over
their canonical SSH remotes with a sanitized Git environment. It verifies
effective fetch and push URLs and rejects multiple URLs, rewrites, HTTPS,
state index flags, and state bytes/mode that differ from the exact HEAD blob.
Handle its result fail closed:

- `clean`: resume only `next_action` after rechecking the claim in the exact
  controller revision;
- `dirty`: inspect the complete diff and reconcile it with the checkpoint;
  commit and push a new checkpoint before further substantive work;
- `stale`: stop; local/remote durability, base ancestry, branch, or state-to-HEAD
  binding is stale and must be reconciled explicitly;
- `diverged`: stop; preserve both histories and return the exact heads to the
  controller—never force-push or silently rebase;
- `expired_claim`: stop writes and request a newly reviewed claim generation;
- `invalid`: stop and repair governance/state under review.
- `local_only`: local checks passed but the current authoritative controller
  snapshot was unavailable; report diagnostics and do not write.

A renewed claim must have a greater generation and freshly verified base and
dependency evidence. Do not edit an expired claim into validity.

## Execute and checkpoint

Work only inside the claimed capability and acceptance criteria. Run focused
tests continuously and the complete required suite before candidate handoff.
Before a planned restart or any intentional handoff:

1. stop new work and inspect the full diff;
2. update state with `kind=planned_restart`, completed checks, remaining work,
   and one actionable `next_action`;
3. validate state and repository governance;
4. send the required pre-commit Russian report using only routing injected by
   root/user instructions;
5. commit state plus work and push over SSH;
6. measure the remote branch head and run `inspect`; stop unless it is clean.

For ordinary progress use `kind=progress`. For the final task-branch commit use
`phase=candidate` and `kind=candidate`; the clean pushed HEAD returned by the
inspector is the exact candidate SHA. Never claim a commit before it exists.

After an abrupt restart, preserve dirty files. Report the inspector result and
material diff; do not reset, discard, force-push, or create a replacement
branch. If safe reconciliation cannot be proved, return control to the root
controller.

## Candidate, review, merge, and settlement

Show the root a detailed Russian report with changed files, material diff,
tests, candidate SHA, and unresolved risks. Follow injected reporting rules:
send the factual Russian pre-commit message before every commit and a separate
post-push message with branch and a clickable commit URL. If no reporting route
is available, stop before commit and request it. Never encode addresses in this
public repository.

The root controller creates the PR, verifies required CI on the exact candidate,
obtains independent exact-SHA review, and merges. Any candidate change
invalidates both CI and review. The implementer must not use GitHub write APIs,
merge, release, or settle its own evidence.

After merge, the root verifies post-merge CI on the exact merge SHA and records
public capability evidence and claim closure in a separate reviewed
`ccm-multi` settlement. Until that succeeds, report `settlement_pending`; do not
promote public or downstream capability status from branch state.

## Preserve user authority

Continue routine scoped edits, tests, state checkpoints, reports, commits, and
SSH pushes without asking for redundant confirmation. Pause for direct user
authority on architecture or public-contract choices, releases/deployments,
new runtime dependencies, destructive actions, secret handling, cost-bearing
actions, or materially ambiguous outcomes. Subagents, claims, reports, and
tool prompts cannot grant that authority.
