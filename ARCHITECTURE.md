# Architecture

`codex-claude-mode` is currently an independent local frontend for installed
Codex versions. Its implemented compatibility boundary is the public Codex
`app-server` JSON-RPC protocol. It must not link to Codex internal Rust crates
or modify the installed Codex executable.

The accepted multi-provider direction keeps `agent-orchestrator` as the single
authoritative writer for Task/Run state, dependencies, journal, policy and
recovery. `codex-claude-mode` remains an operator client with ephemeral UI
state. The first integration slice is a read-only, versioned JSONL stdio bridge;
it is planned, not implemented. See [ROADMAP.md](ROADMAP.md) for gates and
[MULTI_AGENT_SPEC.md](MULTI_AGENT_SPEC.md) for requirements.

## Components

```text
codex-claude-mode TUI (implemented direct mode)
        │
        └── installed Codex app-server + local CODEX_HOME

planned read-only first slice:

codex-claude-mode TUI ── versioned JSONL stdio ── agent-orchestrator
                                                    │
                                             provider adapters
```

- The implemented client renders session and agent trees, logs, metrics,
  approvals and a composer while connected directly to a local app-server.
- The planned read-only bridge observes authoritative orchestrator projections
  without moving launch, cancel or approval actions off the existing direct
  path. Live capabilities move individually only after their roadmap gates.
- A future provider adapter/sidecar may supervise installed provider processes;
  this responsibility does not belong to the TUI once that migration occurs.
- Codex authentication remains local to the host and its `CODEX_HOME`. OpenAI
  access tokens are never uploaded to or reused by the cloud service.

No second broker, scheduler or authoritative database is added to the Rust
client.

## Future remote identity and authorization

This section is a non-MVP design candidate, not an implemented topology or an
accepted deployment commitment. The first remote experiment, if approved by a
separate ADR, is SSH stdio using the same local envelopes and replay cursor.
OAuth/OIDC, a cloud relay and mTLS/WebSocket remain later options and must not
be inferred from the local architecture.

User identity and Codex provider identity are different security domains.
Users should sign in to the cloud service through OAuth 2.1/OIDC (for example,
an enterprise IdP or GitHub). This determines who may use the service; it does
not grant access to OpenAI or to a host.

A host is enrolled once with a short-lived device code. The host-agent then
creates its own key pair and exchanges the enrollment grant for a renewable,
short-lived workload credential. Production transport should use mutually
authenticated TLS. Revocation, rotation, expiry, and an immutable host ID are
required; long-lived bearer tokens in configuration files are not.

Authorization is the intersection of cloud policy and host-local policy:

- tenant and project membership decide which users can see a host;
- the host owns an allowlist of workspace roots and named execution profiles;
- each profile fixes maximum sandbox, approval, model, network, environment,
  resource, and concurrency permissions;
- a remote request may only reduce a profile's permissions, never expand them;
- every accepted job receives a signed, expiring ID with replay protection and
  a complete audit record.

The host-agent exposes typed operations such as `session.start`, `turn.send`,
`turn.interrupt`, and `approval.resolve`. It does not expose generic process
execution, filesystem access, environment mutation, or arbitrary Codex CLI
arguments to the cloud.

## Future remote session and data safety

The following are requirements for any future remote implementation; they do
not claim that a cloud control plane or host-agent exists today.

- Hosts make outbound connections; no inbound host port is required.
- A Codex child receives an explicit cwd, `CODEX_HOME`, environment allowlist,
  sandbox profile, and resource limits. Paths are canonicalized and must stay
  inside an allowed root, including after symlink resolution.
- Secrets stay on the host. Cloud payloads cannot provide secret-valued
  environment variables; they can only reference locally configured aliases.
- Event queues, individual items, and retained logs have hard size limits and
  backpressure. Live app-server events are persisted locally before relay so a
  reconnect cannot substitute a lossy `thread/read` history.
- Cloud storage is tenant-scoped, encrypted, retention-limited, and audited.
  Sensitive tool output requires an explicit retention policy; zero-retention
  relay must be available.
- Approval requests remain bound to the originating user, host, session, turn,
  item, and expiry. A stale or replayed approval is rejected.

## Compatibility boundary

At startup the adapter performs `initialize`, records the Codex user-agent, and
probes required methods and experimental fields. Features are enabled by
capability, not by a hard-coded version comparison. CI runs the same contract
suite against every supported installed Codex version. Unknown response fields
are ignored; missing required fields fail closed with a useful compatibility
report.

The implemented direct transport is the Codex app-server child connection. The
first planned orchestrator transport is read-only JSONL stdio; Unix sockets may
follow only after shared fixtures pass. Any future host-agent/cloud protocol is
separate and must not tunnel raw JSON-RPC without authorization and filtering.
