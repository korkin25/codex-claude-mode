# Task status

Open work for this repository. Ordering and rationale are in
[ROADMAP.md](ROADMAP.md). Statuses are `done`, `ready`, `planned` and
`blocked`. `done` requires verifiable evidence; code in a dirty worktree is not
evidence.

| ID | Status | Lane | Task | Depends on |
|---|---|---|---|---|
| `CCM-SERVE-001` | `ready` | `core` | Headless `serve`: own the app-server connection, keep the projection, expose a `0600` Unix socket under `$XDG_RUNTIME_DIR` | — |
| `CCM-SERVE-002` | `planned` | `core` | TUI becomes a `serve` client; `--direct` preserves current single-process behavior | `CCM-SERVE-001` |
| `CCM-CTL-001` | `planned` | `core` | `codex-claude-mode ctl` with `--json`: list, new, send, read, approve | `CCM-SERVE-001` |
| `CCM-SKILL-001` | `planned` | `core` | `skills/` entry driving `ctl` for a host-local agent framework | `CCM-CTL-001` |
| `CCM-DAEMON-001` | `planned` | `optional-daemon` | Optionally connect `serve` to `codex app-server daemon` so agents survive a `serve` restart | `CCM-SERVE-001` |
| `CCM-DIRECT-001` | `ready` | `direct-compatibility` | Split and verify the direct-Codex compatibility backlog: reconnect/resume, paginated history, multi-root selection, runtime schema probing | — |
| `CCM-PROMPT-001` | `done` | `maintenance` | Fix empty form elicitation confirmation: an empty standard form offers accept without relaxing non-empty or `openai/form` handling | — |

## Acceptance notes

### CCM-SERVE-001

Socket path, permissions and cleanup are tested; a stale
  socket from a dead `serve` is detected rather than reused; the projection
  reconnects from a monotonic cursor and forces a fresh snapshot when the
  cursor is too old.

### CCM-SERVE-002

The TUI renders identically over the socket and in
  `--direct`; losing `serve` produces a visible degraded state, not a silent
  idle one.

### CCM-CTL-001

Every command has stable `--json` output; approvals bind the
  approval ID, request digest, expiry and an idempotency key, and reject stale,
  altered or replayed decisions.

### CCM-SKILL-001

The skill shows the raw diff for an approval rather than a
  summary, and never answers an approval on its own.

### CCM-DAEMON-001

Requires `codex app-server daemon enable-remote-control`;
  falls back to a spawned child when the daemon is unavailable.

### CCM-DIRECT-001

Reconnect/resume, paginated history, multi-root selection and runtime schema
probing each receive focused compatibility tests without depending on `serve`.

### CCM-PROMPT-001

An empty-object standard form shows `[y] accept`; `y` and
  the default Enter choice return `{"action":"accept","content":{}}`. URL
  elicitation still accepts with null content, while non-empty standard forms
  and `openai/form` remain fail-closed until the TUI can collect their fields.
  Focused prompt tests cover every one of these cases.

`CCM-PROMPT-001` was delivered by [PR #4](https://github.com/korkin25/codex-claude-mode/pull/4)
at merge commit [`2c5382035ecf84724fe796332ed1c252f1fb0bce`](https://github.com/korkin25/codex-claude-mode/commit/2c5382035ecf84724fe796332ed1c252f1fb0bce);
the [Linux/macOS CI run](https://github.com/korkin25/codex-claude-mode/actions/runs/31911990635)
completed successfully on its exact PR head.

## Rules

- Identifiers are `CCM-<AREA>-<n>`; numbers are never reused or renumbered.
- Completed work moves to [CHANGELOG.md](CHANGELOG.md) with its evidence.
- Capability status must agree with
  [`delivery/capabilities.json`](delivery/capabilities.json); only `verified`
  manifest entries constitute reusable capability evidence.
