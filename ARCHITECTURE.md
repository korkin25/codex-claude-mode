# Architecture

`codex-claude-mode` is an independent frontend and, later, a remote control
plane for installed Codex versions. Its compatibility boundary is the public
Codex `app-server` JSON-RPC protocol. It must not link to Codex internal Rust
crates or modify the installed Codex executable.

## Components

```text
terminal/web client
        |
        | OAuth/OIDC user session
        v
cloud control plane and event relay
        ^
        | outbound mTLS/WebSocket, host identity
        |
host-agent ── local policy engine ── installed codex app-server
                                      local CODEX_HOME
```

- The client renders session and agent trees, logs, metrics, approvals, and a
  composer. The current terminal client can also connect directly to a local
  `app-server` child without any cloud component.
- The cloud control plane authenticates users, authorizes access to registered
  hosts, schedules typed jobs, and relays bounded event streams. It never sends
  arbitrary shell commands to the host-agent.
- The host-agent keeps an outbound connection to the cloud, validates every
  requested job against local policy, and supervises one or more installed
  `codex app-server` processes.
- Codex authentication remains local to the host and its `CODEX_HOME`. OpenAI
  access tokens are never uploaded to or reused by the cloud service.

## Identity and authorization

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

## Session and data safety

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

The first supported transport is local stdio. Unix sockets are the next local
transport; the host-agent/cloud protocol is separate and must not tunnel raw
JSON-RPC without authorization and filtering.

