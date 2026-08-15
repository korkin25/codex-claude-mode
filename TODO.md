# Task status

Open work for this repository. Ordering and rationale are in
[ROADMAP.md](ROADMAP.md). Statuses are `done`, `ready`, `planned` and
`blocked`. `done` requires verifiable evidence; code in a dirty worktree is not
evidence.

| ID | Status | Task | Depends on |
|---|---|---|---|
| `CCM-SERVE-001` | `planned` | Headless `serve`: own the app-server connection, keep the projection, expose a `0600` Unix socket under `$XDG_RUNTIME_DIR` | — |
| `CCM-SERVE-002` | `planned` | TUI becomes a `serve` client; `--direct` preserves current single-process behavior | `CCM-SERVE-001` |
| `CCM-CTL-001` | `planned` | `codex-claude-mode ctl` with `--json`: list, new, send, read, approve | `CCM-SERVE-001` |
| `CCM-SKILL-001` | `planned` | `skills/` entry driving `ctl` for a host-local agent framework | `CCM-CTL-001` |
| `CCM-DAEMON-001` | `planned` | Optionally connect `serve` to `codex app-server daemon` so agents survive a `serve` restart | `CCM-SERVE-001` |
| `CCM-DIRECT-001` | `planned` | Split and verify the direct-Codex compatibility backlog: reconnect/resume, paginated history, multi-root selection, runtime schema probing | — |

## Acceptance notes

- `CCM-SERVE-001` — socket path, permissions and cleanup are tested; a stale
  socket from a dead `serve` is detected rather than reused; the projection
  reconnects from a monotonic cursor and forces a fresh snapshot when the
  cursor is too old.
- `CCM-SERVE-002` — the TUI renders identically over the socket and in
  `--direct`; losing `serve` produces a visible degraded state, not a silent
  idle one.
- `CCM-CTL-001` — every command has stable `--json` output; approvals bind the
  approval ID, request digest, expiry and an idempotency key, and reject stale,
  altered or replayed decisions.
- `CCM-SKILL-001` — the skill shows the raw diff for an approval rather than a
  summary, and never answers an approval on its own.
- `CCM-DAEMON-001` — requires `codex app-server daemon enable-remote-control`;
  falls back to a spawned child when the daemon is unavailable.

## Rules

- Identifiers are `CCM-<AREA>-<n>`; numbers are never reused or renumbered.
- Completed work moves to [CHANGELOG.md](CHANGELOG.md) with its evidence.
