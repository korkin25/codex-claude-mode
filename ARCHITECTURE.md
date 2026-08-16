# Architecture

`codex-claude-mode` is a local frontend for installed Codex versions. Its
compatibility boundary is the public Codex `app-server` JSON-RPC protocol. It
must not link to Codex internal Rust crates or modify the installed Codex
executable.

## Components

```text
codex-claude-mode TUI (implemented direct mode)
        │
        └── installed Codex app-server + local CODEX_HOME
```

The client renders session and agent trees, logs, metrics, approvals and a
composer while connected directly to a local app-server. It spawns the
app-server as a child process and owns that connection for its lifetime.

Codex authentication remains local to the host and its `CODEX_HOME`. This
project does not provide separate authentication and never copies OpenAI access
tokens anywhere.

## Planned split: `serve` and `ctl`

The single-process design ties every session to the lifetime of one terminal.
The planned change separates state ownership from rendering:

```text
installed Codex app-server
        │
codex-claude-mode serve        headless: owns the app-server connection,
        │                      keeps the projection, exposes a local
        │                      Unix domain socket (mode 0600)
        ├── codex-claude-mode  TUI client (`--direct` keeps today's behavior)
        └── codex-claude-mode ctl --json
```

`serve` is the only writer to the app-server connection, so the TUI and any
`ctl` client observe the same sessions. The socket is filesystem-scoped to the
invoking user under `$XDG_RUNTIME_DIR`; nothing binds to a network interface.

Socket notifications are wake-up hints, not durable truth. Before publishing
an observation from its direct app-server connection, `serve` records it in a
bounded local observation journal. Clients resume from its monotonic cursor;
if retention has removed a requested interval, they replace their projection
from a fresh provider snapshot and display the history gap. This journal
records only what the local gateway observed. It does not create task, run,
policy or approval authority.

`serve` owns direct-mode process probes, heartbeat timers and reconciliation.
An OS service supervisor may keep `serve` alive, but neither a TUI/`ctl` client
nor periodic model turns determine liveness. tmux may remain a human attach
convenience; it is not a provider-state or approval source.

A later optional step connects `serve` to `codex app-server daemon` instead of
spawning its own child, so agents survive a `serve` restart. That path requires
`codex app-server daemon enable-remote-control` and is not required for the
split above.

## Remote operation

Remote access is not implemented by this project and no listener is provided.
The supported arrangement is an agent framework running on the same host that
invokes `ctl` locally; the transport to a phone or another machine belongs to
that framework, not here.

Any future remote path must keep these properties:

- approvals bind to the approval ID, a digest of the original request, an
  expected policy revision, expiry, authenticated principal and idempotency
  key, so a stale, altered or replayed decision is rejected;
- read-only observation and mutating actions are separately authorized;
- reconnect resumes from a monotonic cursor with bounded retention and
  backpressure, and an expired cursor forces a fresh snapshot instead of a
  guess.

An agent framework has no standing approval authority. It may carry only an
operator's explicit decision after showing the raw request digest and diff;
the executor validates the immutable authorization record immediately before
the effect. An approved request records authorization, not successful
execution.

## Compatibility boundary

At startup the adapter performs `initialize`, records the Codex user-agent, and
probes required methods and experimental fields. Features are enabled by
capability, not by a hard-coded version comparison. CI runs the same contract
suite against every supported installed Codex version. Unknown response fields
are ignored; missing required fields fail closed with a useful compatibility
report.

The implemented transport is the Codex app-server child connection. Unknown
events and approval kinds fail closed rather than being guessed.

## Delivery contract

The public capability DAG is published in
[`delivery/capabilities.json`](delivery/capabilities.json). It describes only
interfaces owned by this repository. A capability being `planned` or `ready`
does not assert that its runtime exists. `verified` requires immutable merge
and CI evidence as defined by the public schema; downstream consumers remain
responsible for checking that evidence independently.
