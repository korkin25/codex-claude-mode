# Atomic task registry

Единственный источник статуса атомарных roadmap-задач. Порядок фаз и gates — в
[ROADMAP.md](ROADMAP.md), требования — в
[MULTI_AGENT_SPEC.md](MULTI_AGENT_SPEC.md). Stable ID не переиспользуются и не
перенумеровываются.

Статусы: `done`, `ready`, `planned`, `blocked`, `deferred`. `done` требует
ссылки на проверяемое evidence; наличие кода в dirty worktree не является
evidence релиза.

## P0 — Baseline freeze

### CCM-BASE-001 — Extract the standalone frontend

- Repo owner: `CCM`
- Status: `done`
- Phase: `P0`
- Depends on: none
- Outcome: независимый frontend запускает установленный Codex app-server без
  внутренних Codex crates.
- Acceptance: отдельная сборка и явные `--codex`, `--codex-home`, `--cwd`.
- Evidence: commits `b0bb1b1`…`4f8fedf`; shipped baseline описан в README.
- Size: `L` (historical)

### CCM-BASE-002 — Deliver the first local workspace slice

- Repo owner: `CCM`
- Status: `done`
- Phase: `P0`
- Depends on: `CCM-BASE-001`
- Outcome: Main/sub-agent UI, независимые логи, composer, prompts и approvals.
- Acceptance: выбранный thread управляется без смешивания логов; unknown
  approvals fail closed.
- Evidence: commits до `4f8fedf`; существующие Rust tests.
- Size: `L` (historical)

### CCM-BASE-003 — Inventory and verify the dirty 0.4.7 candidate

- Repo owner: `CCM`
- Status: `done`
- Phase: `P0`
- Depends on: `CCM-BASE-002`
- Outcome: таблица `shipped | tested | unverified | planned` для накопленных
  direct-Codex UX изменений.
- Acceptance: версия не объявлена готовой; каждый пункт сопоставлен с test,
  commit/release evidence либо отдельной задачей.
- Evidence: [BASELINE_INVENTORY.md](BASELINE_INVENTORY.md); packaged Linux
  `0.4.7` hashes and timestamp recorded separately from the post-package dirty
  tree; 72 baseline tests pass. `G0` remains open because macOS evidence is not
  yet available.
- Size: `S`

### CCM-BASE-004 — Commit a verified direct-mode baseline

- Repo owner: `CCM`
- Status: `done`
- Phase: `P0`
- Depends on: `CCM-BASE-003`
- Outcome: накопленные изменения разделены на атомарные проверенные коммиты, а
  рабочее дерево возвращено в чистое baseline-состояние.
- Acceptance: application и documentation changes зафиксированы отдельно;
  каждый application commit компилируется и проходит релевантные tests;
  финальный baseline проходит Rust 1.95 fmt, tests и Clippy; `dist` не попадает
  в Git; релиз `0.4.8` не создаётся в рамках этой задачи.
- Evidence: application commit `4234d55`; Rust 1.95.0 `cargo fmt --check`, 72
  tests and `cargo clippy --all-targets -- -D warnings` pass on Linux;
  `git diff --check` passes. Documentation is committed separately.
- Size: `M`

### CCM-DIRECT-001 — Split and verify direct-Codex compatibility backlog

- Repo owner: `CCM`
- Status: `planned`
- Phase: `P0`
- Depends on: `CCM-BASE-003`
- Outcome: отдельные задачи на reconnect/resume, paginated history, multi-root
  selection и runtime schema probing.
- Acceptance: каждая capability имеет собственный test/evidence; unknown
  events и approvals fail closed; CLI output не парсится вместо app-server.
- Evidence: pending.
- Size: `M`

### CCM-DIRECT-002 — Recover sessions safely across workspaces

- Repo owner: `CCM`
- Status: `done`
- Phase: direct-mode compatibility patch `0.4.12`
- Depends on: `CCM-BASE-004`
- Outcome: explicit thread selection never falls back to creating a replacement;
  recovery exposes saved workspace context, uses the chosen `cwd`, and loads all
  bounded pages of roots and descendants. The TUI also exposes `CODEX_HOME`, a
  terminal-safe clipboard shortcut, and preserves tail-follow after approvals.
- Acceptance: transcript coverage proves direct `thread/resume`, no hidden
  `thread/start`, current/saved/deleted/Trash workspace handling, interleaved and
  greater-than-200 pagination, plus F6 modal priority and approval tail-follow.
- Evidence: Rust 1.95: 138 unit/integration tests and 5 CLI tests (143 total); `cargo fmt
  --check`, Clippy with warnings denied, and release build passed before commit.
- Size: `M`

### SHARED-PLATFORM-001 — Establish the Linux/macOS baseline matrix

- Repo owner: `SHARED`
- Status: `done`
- Phase: `P0`
- Depends on: `CCM-BASE-002`
- Outcome: обязательная OS/architecture/capability/test/release matrix.
- Acceptance: gaps явно `unsupported/degraded`; product/tests не используют
  Linux-only shell/GNU assumptions; CI plan покрывает обе OS.
- Evidence: [PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md) records the capability
  matrix, source audit, exact required CI jobs and manual smoke checklist.
  Completion is planning evidence only: macOS/ARM64 runtime support remains
  unverified and `G0` stays open.
- Size: `M`

### CCM-CI-001 — Run the baseline CI and capture platform artifacts

- Repo owner: `CCM`
- Status: `done`
- Phase: `P0`
- Depends on: `SHARED-PLATFORM-001`
- Outcome: каждый push и pull request проверяет direct-mode baseline на Linux
  x86_64 и macOS ARM64 закреплённым Rust 1.95.0 и сохраняет именованный
  release-бинарник без публикации GitHub Release.
- Acceptance: обязательные Linux `fmt`, tests, Clippy и release build проходят;
  `macos-14` выполняет те же проверки и явно подтверждает `arm64`; артефакты
  имеют OS/architecture в имени и SHA-256; workflow не использует GNU-only
  shell assumptions.
- Evidence: `.github/workflows/ci.yml`; GitHub Actions run
  [31557720447](https://github.com/korkin25/codex-claude-mode/actions/runs/31557720447)
  на commit `29a7308` успешно выполнил 72 tests, fmt, Clippy и release build на
  Linux x86_64 и macOS ARM64 и сохранил оба именованных binary artifacts с
  SHA-256. Локально те же locked checks проходят на Linux с Rust 1.95.0.
- Size: `S`

### CCM-RELEASE-001 — Publish the verified 0.4.8 baseline

- Repo owner: `CCM`
- Status: `done`
- Phase: `P0`
- Depends on: `CCM-BASE-004`, `CCM-CI-001`
- Outcome: immutable tag `v0.4.8` publishes Linux x86_64 and macOS ARM64
  archives, checksums and concise release notes through GitHub Actions.
- Acceptance: package version and tag match; both platform jobs pass pinned
  Rust 1.95 fmt, tests, Clippy and release build; archives have OS/architecture
  names and a release-level SHA-256 manifest; GitHub Release uses the existing
  repository visibility and no local `dist` files.
- Evidence: annotated tag `v0.4.8` targets green commit `2f45c29`; CI run
  [31561173580](https://github.com/korkin25/codex-claude-mode/actions/runs/31561173580)
  and release run
  [31561226338](https://github.com/korkin25/codex-claude-mode/actions/runs/31561226338)
  pass on Linux x86_64 and macOS ARM64. The non-draft, non-prerelease
  [GitHub Release](https://github.com/korkin25/codex-claude-mode/releases/tag/v0.4.8)
  contains both archives and verified release-level checksums.
- Size: `S`

## P1 — Shared protocol contract

### CCM-REMOTE-PREVIEW-001 — Specify a temporary paired remote observer

- Repo owner: `CCM`
- Status: `done`
- Phase: early preview after `P0` (parallel to `P1/P2`; does not satisfy `P8`)
- Depends on: `CCM-BASE-004`, `CCM-CI-001`
- Outcome: research/ADR for a localhost-only HTTP snapshot+SSE+PWA observer of
  current direct-Codex state through a manually managed Tailscale Serve,
  Cloudflare Tunnel or ngrok TLS tunnel.
- Acceptance: read-only initial scope; paired/revocable bounded device access;
  reconnect cursor, redaction and explicit security non-goals; Linux-first and
  mandatory macOS support; provider-neutral envelopes can migrate to AOR
  without preserving a second source of truth.
- Evidence: [REMOTE_PREVIEW.md](REMOTE_PREVIEW.md); documentation-only change,
  no listener or remote mutation implemented.
- Size: `S`

Follow-up implementation tasks must be created separately for the loopback
observer, paired PWA and cross-platform hardening. They may not add mutations
or claim `G8` without a new security decision.

### SHARED-PROTO-001 — Specify bounded versioned envelopes

- Repo owner: `SHARED`
- Status: `planned`
- Phase: `P1`
- Depends on: `CCM-BASE-003`
- Outcome: JSONL command/event/snapshot schema с sequence, replay cursor,
  correlation/causation, idempotency и expected revision.
- Acceptance: bounds и compatibility/fail-closed rules нормативны и не содержат
  provider-specific IDs как canonical identity.
- Evidence: pending shared schema/ADR.
- Size: `M`

### SHARED-PROTO-002 — Add cross-language golden fixtures

- Repo owner: `SHARED`
- Status: `planned`
- Phase: `P1`
- Depends on: `SHARED-PROTO-001`
- Outcome: одинаковые fixtures lifecycle, gap, duplicate/conflict, reconnect,
  approval и unknown version/type для Python и Rust.
- Acceptance: обе реализации дают одинаковые projections/errors.
- Evidence: pending tests in both repositories.
- Size: `M`

## P2 — Read-only bridge

### AOR-BRIDGE-001 — Expose read-only stdio snapshot/replay

- Repo owner: `AOR`
- Status: `planned`
- Phase: `P2`
- Depends on: `SHARED-PROTO-002`
- Outcome: `serve --stdio` над authoritative AOR projections.
- Acceptance: snapshot+ordered replay, bounded backpressure, explicit protocol
  errors; отсутствуют mutating commands.
- Evidence: pending AOR integration tests.
- Size: `M`

### CCM-BRIDGE-001 — Render neutral read-only projections

- Repo owner: `CCM`
- Status: `planned`
- Phase: `P2`
- Depends on: `SHARED-PROTO-002`, `AOR-BRIDGE-001`
- Outcome: provider-neutral Agent/Task/Run/Approval/Artifact views и reconnect.
- Acceptance: direct-Codex режим сохранён; draft/scroll/unread не теряются;
  disconnected/stalled/degraded показаны честно.
- Evidence: pending integration and snapshot tests.
- Size: `L`

## P3 — Codex shadow

### SHARED-SHADOW-001 — Compare Codex and AOR projections

- Repo owner: `SHARED`
- Status: `planned`
- Phase: `P3`
- Depends on: `CCM-BRIDGE-001`
- Outcome: direct Codex events поступают в shadow normalization без управления
  действиями.
- Acceptance: replay/dedupe/crash scenarios совпадают; payload bounded.
- Evidence: pending cross-repo integration tests.
- Size: `L`

## P4 — Live Codex slices

### AOR-CODEX-001 — Enable Observe as the first live capability

- Repo owner: `AOR`
- Status: `blocked`
- Phase: `P4`
- Depends on: `SHARED-SHADOW-001`, explicit owner approval for AOR public API
- Outcome: только `Observe` проходит authoritative path.
- Acceptance: leases/reconciliation/exactly-one terminal outcome; rollback to
  direct-Codex documented.
- Evidence: pending.
- Size: `M`

### AOR-CODEX-002 — Move Codex process capabilities incrementally

- Repo owner: `AOR`
- Status: `planned`
- Phase: `P4`
- Depends on: `AOR-CODEX-001`
- Outcome: process ownership, launch, cancel и approval включаются отдельными
  audited slices.
- Acceptance: fixture и rollback для каждой capability; effective permissions
  have evidence.
- Evidence: pending.
- Size: `XL`

## P5 — Second provider

### AOR-ADAPTER-CLAUDE-001 — Add the Claude provider adapter

- Repo owner: `AOR`
- Status: `planned`
- Phase: `P5`
- Depends on: `AOR-CODEX-002`
- Outcome: Claude session/run lifecycle через общий adapter contract.
- Acceptance: capability negotiation, cancel/recovery and requested/effective
  permission evidence work on Linux/macOS.
- Evidence: pending adapter contract tests.
- Size: `L`

### CCM-ADAPTER-CLAUDE-001 — Present Claude in the common operator UX

- Repo owner: `CCM`
- Status: `planned`
- Phase: `P5`
- Depends on: `AOR-ADAPTER-CLAUDE-001`
- Outcome: Codex и Claude одновременно видны и управляемы в одном workspace.
- Acceptance: provider/model/status/capabilities/permissions различимы; fallback
  не выдаётся за native capability.
- Evidence: pending integration and snapshot tests.
- Size: `M`

## P6 — Compare and bus

### AOR-COMPARE-001 — Implement bounded two-provider compare

- Repo owner: `AOR`
- Status: `planned`
- Phase: `P6`
- Depends on: `CCM-ADAPTER-CLAUDE-001`
- Outcome: immutable digest-bound TaskSnapshot запускается на Codex и Claude.
- Acceptance: independent latency/usage/cost/outcome/artifacts, partial results,
  all/quorum/deadline, no automatic winner.
- Evidence: pending integration tests.
- Size: `L`

### AOR-BUS-001 — Implement durable scoped agent messaging

- Repo owner: `AOR`
- Status: `planned`
- Phase: `P6`
- Depends on: `CCM-ADAPTER-CLAUDE-001`, `SHARED-PROTO-002`
- Outcome: inbox/outbox, ack/redelivery, dependency wake-up и result routing.
- Acceptance: idempotency, cycle/depth/fan-out/budget limits and immediate
  unblock; provider credentials не передаются.
- Evidence: pending deterministic and recovery tests.
- Size: `XL`

### SHARED-BUS-001 — Expose scoped MCP bus tools and skills

- Repo owner: `SHARED`
- Status: `planned`
- Phase: `P6`
- Depends on: `AOR-BUS-001`
- Outcome: create/send/query/wait/cancel/collect доступны разным agents через
  vendor-specific configuration одного контракта.
- Acceptance: short-lived scoped identity, bounded context and schema-validated
  results; repeated idempotency key не создаёт работу повторно.
- Evidence: pending cross-provider fixtures.
- Size: `L`

## P7 — Optional integrations

### CCM-IDE-001 — Add a thin VS Code/Cursor operator client

- Repo owner: `CCM`
- Status: `deferred`
- Phase: `P7`
- Depends on: `AOR-BUS-001`
- Outcome: Agents/Tasks/Attention/Artifacts используют native Explorer, Search,
  SCM, diff, editor, terminal и workspace APIs.
- Acceptance: typed capability-gated IPC; no arbitrary IDE command/shell; TUI
  minimal viewer remains fallback.
- Evidence: pending.
- Size: `XL`

### SHARED-OBS-001 — Export bounded redacted OTLP telemetry

- Repo owner: `SHARED`
- Status: `deferred`
- Phase: `P7`
- Depends on: `AOR-COMPARE-001`, `AOR-BUS-001`
- Outcome: correlated tasks/runs/provider/tools with token/cost provenance.
- Acceptance: cardinality/privacy bounds; unknown values remain unknown; no
  prompts/code/secrets/raw IDs by default.
- Evidence: pending.
- Size: `L`

### AOR-RAG-001 — Provide shared code-intelligence retrieval

- Repo owner: `AOR`
- Status: `deferred`
- Phase: `P7`
- Depends on: `AOR-BUS-001`
- Outcome: bounded lexical/LSP/SCIP/tree-sitter retrieval with optional existing
  embedding storage, exposed through MCP.
- Acceptance: SHA/worktree/index-generation freshness, ACL/redaction, exact
  citations and explicit degraded fallback; no custom vector DB.
- Evidence: pending.
- Size: `XL`

### AOR-CHAT-001 — Add Telegram and Slack sidecars

- Repo owner: `AOR`
- Status: `deferred`
- Phase: `P7`
- Depends on: `AOR-BUS-001`, authoritative approval lifecycle
- Outcome: opt-in notification/status/result first; approved typed actions later.
- Acceptance: pairing/RBAC, digest/revision/expiry, replay protection, redaction,
  durable retry and mock API tests; chat is not source of truth.
- Evidence: pending.
- Size: `XL`

### AOR-MCP-001 — Add the MCP-first SaaS connector registry

- Repo owner: `AOR`
- Status: `deferred`
- Phase: `P7`
- Depends on: `AOR-BUS-001`
- Outcome: allowlisted versioned profiles for Jira, Confluence and similar SaaS.
- Acceptance: scoped secret refs, read/mutation separation, exact approval,
  freshness/tenant bounds and malicious-content tests; no bespoke core clients.
- Evidence: pending.
- Size: `XL`

## P8 — Remote access

### SHARED-REMOTE-001 — Decide remote trust boundary in an ADR

- Repo owner: `SHARED`
- Status: `deferred`
- Phase: `P8`
- Depends on: `AOR-BUS-001`
- Outcome: accepted identity/RBAC/approval/revocation/retention/path/threat model.
- Acceptance: SSH stdio evaluated first; HTTPS/WSS/listeners explicitly gated.
- Evidence: pending ADR and security review.
- Size: `M`

### SHARED-REMOTE-002 — Prototype SSH stdio

- Repo owner: `SHARED`
- Status: `deferred`
- Phase: `P8`
- Depends on: `SHARED-REMOTE-001`
- Outcome: local envelopes/replay work unchanged over authenticated SSH stdio.
- Acceptance: Linux/macOS client-host matrix, bounded reconnect/backpressure and
  safe remote URI/path handling.
- Evidence: pending integration tests.
- Size: `L`

## Deferred ideas requiring decomposition

До перевода в `planned` этим темам нужны отдельные owner, dependency и bounded
acceptance: Gemini/Grok adapters; structured `@` autocomplete and collectors;
provider-neutral external isolation; intelligent script/impact advisor; project
and worktree forest; bulk session administration; token recommender; optional
AG-UI/A2A federation beyond the scoped bus edge.

## Legacy mapping

Старые номера сохранены только для истории и не являются task identity.

| Legacy TODO | Stable replacement |
|---|---|
| 1 | `CCM-BASE-001` |
| 2 | `CCM-BASE-002` |
| 3 | `CCM-BASE-003`, `CCM-DIRECT-001` |
| 4 | `SHARED-PROTO-001`, `SHARED-PROTO-002` |
| 5 | `AOR-BRIDGE-001` |
| 6 | `CCM-BRIDGE-001` |
| 7 | `SHARED-SHADOW-001` |
| 8 | `AOR-CODEX-001` |
| 9 | `AOR-CODEX-002`, `AOR-ADAPTER-CLAUDE-001`, `CCM-ADAPTER-CLAUDE-001` |
| 10 | `AOR-COMPARE-001` |
| 11 | `AOR-BUS-001`, `SHARED-BUS-001` |
| 12 | `CCM-IDE-001`, `SHARED-OBS-001`, `AOR-RAG-001` |
| 13 | `SHARED-REMOTE-001`, `SHARED-REMOTE-002` |
| 14 | `SHARED-PLATFORM-001` plus every phase gate |
| 15 | `AOR-CHAT-001` |
| 16 | `AOR-MCP-001` |

## Explicit non-goals

- No second broker or authoritative DB in Rust.
- No custom workflow/graph DSL, vector DB, trace backend or discovery protocol.
- No full IDE Explorer/editor/SCM/diff in TUI; its minimal viewer is a fallback.
- No vendor-to-vendor direct messaging, implicit credential sharing, silent
  permission downgrade/escalation, automatic routing or winner selection.
- No bespoke Jira/Confluence/GitHub/GitLab/Linear/Notion core clients; use
  reviewed MCP servers behind the registry.
