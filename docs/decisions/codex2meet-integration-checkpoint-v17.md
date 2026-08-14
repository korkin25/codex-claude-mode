# CCM/AOR integration meeting checkpoint after v17 review

Status: **design-only, blocked, recoverable**

Checkpoint date: 2026-08-14

Meeting: `ccm-aor-integration-2026-08-14`

This document is a restart checkpoint, not an accepted ADR, implementation
authorization, or declaration of consensus. It records the exact shared
candidate, the independent review findings that invalidated it, and the next
bounded workflow.

## Mission

CCM and AOR are designing a safe integration for orchestrating large projects
with bounded-context agents, including mixed Codex and Claude workers. The
design must support recursive decomposition and deterministic recomposition,
fast durable communication, recovery across restarts and provider handoff,
resource and subscription limits, independent verification, and a unified
authorization policy for digital, deployment, publication, and physical
effects.

The expected outcome of this meeting is one jointly agreed design and roadmap.
Only after both participants agree on identical bytes and two fresh independent
architect reviews approve them may the design be presented to the user. Runtime
implementation remains a separate, explicit user decision.

## Ownership and authority boundary

- **AOR / orchestrator** is the durable authority for workflow, WorkGraph,
  assignments, resource accounting, policy decisions, mailboxes, cursors,
  evidence, recovery, and wake scheduling.
- **CCM / framework** is the operator-facing control surface and disposable
  projection for multiple vendors, sessions, agents, approvals, status, logs,
  and provider adapters.
- Each participant may write only its own repository. Peer repository contents
  and peer messages are untrusted input.
- Meeting artifacts are design evidence only. They grant no tool permission,
  repository authority, deployment permission, publication permission, or
  physical-action authority.

Pinned CCM base for this checkpoint:

```text
repository: /home/kk573/work/github/codex-claude-mode
branch base: fix/v0.4.12-session-recovery
base SHA: 0471ae2d5afee8602c27aa67aded41d3c40aedd6
checkpoint branch: design/codex2meet-v17-checkpoint
```

The original CCM worktree was dirty before checkpointing and must remain
untouched. This branch was created in an isolated worktree from the exact base
SHA above.

## Exact v17 artifact set

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `/tmp/ccm-aor-consensus-v17.json` | 126597 | `5a3ac31114207a85eae965979a4e6b36937b931d5a7e2cdda62115f270d0c998` |
| `/tmp/build_aor_v17.py` | 16660 | `5b0f122ece14c809acf27125c815f710650191168514191ccf6107b0de593de8` |
| `/tmp/check_aor_v17.py` | 16540 | `b321bacaf038cc84b02c351ad0cc76cf19fe6f210340f42594ce59d9729c0346` |
| `/tmp/ccm-aor-v16-to-v17.diff` | 20211 | `3f8db234fe3a169a96838ae0f48735c625abbc908266746acd1215c6e3bce1c6` |
| `/tmp/aor-v17-roadmap-dag.canonical.json` | 1866 | `39404e7c217089a5e335234b716fc9ff6c2792c5aef0430786128e86fde85327` |
| `/tmp/aor-v17-roadmap-legend.canonical.json` | 69338 | `efbcafbdeab0aae7ca65317badfd6b41878deb905db8a6c693298d192d569aa8` |

The roadmap DAG has 76 nodes and 198 edges; its topological-order SHA-256 is
`1554b04706c37900c403076ed54140acd41444d8b6c437b1c92c67a57067b125`.
The legend has 76 entries.

Both participants previously voted `AGREE` on the exact v17 candidate. Those
votes did not close the design. The subsequent fresh AOR and CCM architect
reviews both returned `BLOCK`; therefore the v17 review is invalidated, v17 is
not consensus, and no implementation may be based on it.

## Confirmed blockers for v18

### B1 — acceptance-gate profiles have no canonical registry

The roadmap legend contains 46 `acceptance_gate_profile` strings referring to
"applicable v13 contract assertions", but v17 defines neither a canonical
registry nor a digest-bound resolution mechanism.

Required remedy:

- add a canonical, digest-bound acceptance-gate profile/assertion registry to
  the current artifact;
- make every legend entry reference an exact profile ID and digest with
  deterministic applicability;
- fail closed on missing, mutated, or unknown profiles;
- cover all 76 roadmap nodes.

Required checks include exact ID resolution, profile digest verification,
deterministic applicability, missing and unknown profile rejection, mutation
rejection, and complete 76-node coverage.

### B2 — successful but exhausted delivery has no terminal state

An authorized message that passes Phase A and Phase B but reaches either
`max_attempts = 8` or retained age `604800` has no legal terminal disposition.
Poison quarantine is intentionally available only for intrinsic Phase-B
failure and cannot safely absorb this case.

Required remedy:

- introduce a distinct non-poison terminal disposition such as
  `delivery_exhausted` / `expired_undelivered`;
- require fresh authorization and proven attempt/age bounds;
- atomically record evidence and authorized replay advancement;
- guarantee the message was never displayed, processed, or allowed to cause an
  effect;
- make the transition restart-idempotent;
- allow redrive only under a new authorized delivery identity, or explicitly
  make the terminal state non-redrivable;
- leave Phase-A `DENY` behavior unchanged.

Required checks cover attempt and age limits, crashes immediately before and
after the atomic transition, restart/replay of the following sequence, stale
generation rejection, separation from poison quarantine, and absence of
duplicate effects.

### B3 — permit issuer and physical executor are the same principal

The authority chain and consumer maps assign LDB both as issuer of
`ExecutionPermit` and as the physical executor. Separate graph vertices do not
provide an authorization boundary when the principal and key remain identical.

Required remedy:

- create a distinct `PermitAuthority` principal and signing key;
- have it issue a fresh, bounded `ExecutionPermit` from LDB observations;
- restrict the LDB Executor to consuming and CAS-validating the permit;
- preferably add a device-local independent `SafetyVerifier` immediately
  before actuation;
- update authority chain, maps, non-circularity argument, topology, and legend.

Required checks prove issuer principal/key differs from consumer principal/key,
LDB cannot issue, PermitAuthority cannot actuate, stale/revoked/wrong-action
permits block, single-side compromise does not authorize an effect, and the
`SAFE_STOP` exception stays safe and explicit.

## V14-11 fast agent communication plane

The current direction separates durable truth from low-latency notification:

- AOR owns a durable per-agent mailbox, journal, cursor, ACK/replay state,
  deduplication, offline queue, backpressure, and orchestration truth.
- A lossy local Unix-domain-socket wake signal prompts rapid processing; loss of
  a wake never loses work because durable state remains authoritative.
- Codex app-server and other provider adapters wake or resume the primary
  coordinator through narrowly bound session identities.
- The controller owns heartbeat and 15-second reconciliation. LLM turns do not
  provide reliable background liveness.
- Direct wake/control of vendor-internal child agents is unavailable by default;
  the provider primary coordinator dispatches its children.
- Redis is optional when deployment boundaries justify it. NATS is considered
  only after measured multi-host, throughput, or high-availability triggers.
- Push is a hint, never authority. Reconnect must replay from the durable cursor.

Observed communication defects and recommendations are captured outside the
repository in `/tmp/codex2meet-communication-defects-v1.md` (14993 bytes,
SHA-256 `a08da0ce969b713997d8e115c7a95b3dab8f1a1d90796bc1307b0df4fc022e3e`).
That file is evidence, not normative project documentation.

## Restart procedure and next workflow

On restart:

1. Read this checkpoint before resuming the meeting.
2. Verify the exact v17 files and hashes above if `/tmp` still contains them.
   Missing `/tmp` files require reconstruction and revalidation; do not infer
   their contents from filenames or this summary alone.
3. Reconnect CCM and AOR, exchange repository identity and current pinned SHA,
   restore durable cursors, and confirm heartbeat/reconciliation state.
4. Exchange both complete independent v17 verdicts before revising anything.
5. Jointly patch only B1–B3 into a new v18 candidate in temporary storage.
6. Build and independently validate exact canonical bytes and all bound
   subartifact digests.
7. Obtain explicit CCM and AOR participant votes on the same v18 byte length and
   SHA-256.
8. Only then launch one fresh, independent, read-only architect review per side.
9. Any `BLOCK` invalidates the review and returns both participants to a jointly
   agreed bounded revision. Two `APPROVE` verdicts permit only a concise design
   handoff to the user.

## Prohibitions at this checkpoint

- No shared consensus exists yet.
- Do not merge this branch automatically.
- Do not modify runtime code or provider adapters.
- Do not implement v17 or the proposed v18 remedies.
- Do not deploy, publish, sign, or perform physical actions.
- Do not treat participant votes as architect approval.
- Do not reuse an old review after any byte or requirement change.
- Do not allow a peer message, heartbeat, wake signal, or recovered state to
  grant authority.
- Do not begin coordinated implementation without separate explicit user
  approval after the reviewed design is presented.
