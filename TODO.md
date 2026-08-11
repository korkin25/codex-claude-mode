# TODO

## Local standalone product

### [x] 1. Create the independent project

Move the standalone frontend to `codex-claude-mode`, remove internal Codex
crate dependencies, launch an installed `codex app-server`, and keep the fixed
test home at `/home/kk573/tmp/codex-agent-picker-test-home`.

### [x] 2. Implement the first local workspace slice

Render Main and nested sub-agents in a horizontal selector, independently
selected borderless logs, status/time/token metrics, keyboard and mouse
navigation, a three-row composer with input history, live assistant output, and
parent-routed sub-agent messages. Discover persisted descendants through the
app-server ancestor filter. Add a non-interactive backend compatibility probe.

### [ ] 3. Complete local app-server behavior

- [x] Implement fail-closed typed command/file/permission approvals, user-input
  and MCP elicitation UI, plus active-turn interrupts.
- [ ] Add reconnect/resume, durable local event journals, paginated history,
  multiline composer editing, correct wrapped-line scrolling, and explicit
  selection among multiple roots.

Never silently accept an approval.

### [ ] 4. Add the compatibility contract

Probe methods/fields at runtime, emit a compatibility report, add fixture tests
for supported schemas, and run end-to-end tests against a matrix of installed
Codex binaries. Package the frontend without bundling Codex.

## Remote access

### [ ] 5. Build the host-agent supervisor

Add workspace-root and profile configuration, safe path resolution, Codex child
lifecycle management, bounded event journals, resource/concurrency limits, and
an outbound-only control connection. Keep Codex credentials local.

### [ ] 6. Define the typed remote protocol

Version commands and events for host enrollment, capabilities, session start,
turn input/interrupt, approvals, agent trees, logs, metrics, reconnect cursors,
and heartbeats. Add idempotency, expiry, replay protection, backpressure, and
hard payload limits. Do not expose arbitrary shell or raw app-server RPC.

### [ ] 7. Implement separate user and host identity

Use OAuth 2.1/OIDC for users. Use one-time device enrollment followed by
short-lived mTLS workload credentials for hosts. Add tenant/project RBAC,
credential rotation and revocation, and auditable authorization decisions.
Do not reuse Codex/OpenAI credentials as service identity.

### [ ] 8. Implement a minimal control plane

Add tenant-isolated persistence, host presence, job dispatch, encrypted event
relay, reconnect cursors, retention controls, rate limits, and audit logs. Begin
with a single-tenant deployment while keeping tenant IDs mandatory in storage
and authorization boundaries.

### [ ] 9. Add remote clients

Connect the terminal UI through the control plane, then add an optional web UI.
Preserve the same tree/log/composer semantics locally and remotely. Clearly
show host, workspace, profile, sandbox, approval mode, and connection state.

### [ ] 10. Security validation before multi-user release

Threat-model tenant escape, confused deputy behavior, symlink/path traversal,
credential theft, replay, event injection, stale approvals, log leakage, denial
of service, and unsafe upgrades. Add adversarial tests, external review,
incident controls, backup/restore tests, and documented key rotation.
