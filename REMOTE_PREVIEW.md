# Temporary remote preview ADR

Status: accepted research decision (`CCM-REMOTE-PREVIEW-001`). This document
does not authorize a network listener or remote mutation capability.

## Decision

Before the authoritative `agent-orchestrator` bridge is ready,
`codex-claude-mode` may expose a narrow, read-only browser preview through a
separate localhost-only gateway. The gateway projects existing direct-Codex UI
state; it does not become a broker, scheduler, provider adapter or source of
truth.

```text
Codex app-server -> codex-claude-mode -> bounded projection
                                           |
                                  loopback HTTP gateway
                                  snapshot + SSE + PWA
                                           |
                         manually managed external TLS tunnel
                                           |
                                     paired browser
```

The process binds only to `127.0.0.1` and `::1` on an explicit or randomly
allocated port. It has no `0.0.0.0` mode. TLS and Internet reachability are
provided by an external, user-started Tailscale Serve, Cloudflare Tunnel or
ngrok process. Raw Codex app-server sockets are never published.

## Protocol boundary

The initial API is deliberately small:

- `GET /v1/meta` returns protocol/build/capability information;
- `GET /v1/snapshot` returns a bounded redacted workspace projection and its
  monotonic cursor;
- `GET /v1/events?cursor=<sequence>` streams ordered Server-Sent Events;
- `POST /v1/pair/exchange` consumes a one-time pairing secret.

Device listing, local confirmation and revocation use TUI/internal IPC, not an
HTTP route: tunnel traffic also arrives from loopback and therefore cannot be
distinguished safely by source address alone.

REST plus SSE is preferred to WebSocket for this preview: browser reconnect,
proxy compatibility and `Last-Event-ID` are sufficient for snapshot/delta
observation. Terminal, live composer and other bidirectional high-frequency
features require a later transport decision.

Every snapshot/event envelope uses versioned provider-neutral names compatible
with the planned shared protocol: `schemaVersion`, `sequence`, `eventId`,
`occurredAt`, `workspaceId`, `type` and bounded `payload`. Direct Codex thread
IDs remain external bindings, not future canonical Task/Run identity.
Reconnect resumes from a monotonic cursor. An expired cursor yields
`410 snapshot_required` with the oldest available sequence; the browser obtains
a new snapshot instead of guessing missing state.

## Pairing and authentication

The local operator requests a one-use random secret of at least 128 bits with a
short expiry. It is displayed as a QR code/link whose secret is carried in the
URL fragment so it does not enter tunnel access logs. Exchange creates a
pending device; the TUI shows device name, fingerprint, requested scope and
approximate network provenance before local confirmation.

Successful pairing returns one opaque device token. Only a salted token hash is
stored locally. Tokens are scoped to `observe`, a workspace, protocol revision
and expiry, and can be revoked locally. Tunnel identity headers may add audit
context only when received from an explicitly configured trusted loopback
proxy; they never replace application pairing.

The first slice has no remote approval, task, message, permission or process
mutation. A later approval slice needs its own gate and must bind a decision to
approval ID, canonical request digest, request/policy revision, expiry,
authenticated device and idempotency key. Stale, changed, expired, replayed or
already resolved requests fail closed; `full access` remains local-only until a
separate security decision.

## Bounds and redaction

Normative starting limits for the prototype:

- event: 64 KiB; inline log/artifact preview: 16 KiB;
- request body: 256 KiB; at most three SSE connections per device;
- queued events per client: 1,000, then disconnect and cursor replay;
- bounded event retention and paginated collections;
- rate-limited pairing/exchange and authentication failures;
- no environment, credentials, auth headers, raw prompts, arbitrary files,
  unrestricted paths, stack traces or binary artifacts in remote projections.

Redaction occurs before data enters the gateway queue. Browser-side masking is
not a security boundary. Unknown event types, schema versions and content
classifications are omitted with a visible degraded status and local audit
record. Logs and filenames receive explicit classification and field allowlists.

## External tunnel profiles

- Tailscale Serve is the preferred closed preview inside a tailnet.
- Cloudflare Tunnel with Access is the preferred clientless browser preview.
- ngrok with OAuth/OIDC and an exact user allowlist is acceptable for a
  short-lived demonstration.
- Quick/public tunnels without identity policy are not supported. Tailscale
  Funnel requires the same application pairing and is not the default.

The application may print validated example commands and health checks, but it
does not install, authenticate or silently start third-party tunnel binaries.
Their lifecycle, account policy, TLS and public hostname remain user-managed.

## Platform contract

Linux x86_64 is the first implementation and release priority. macOS ARM64 is a
mandatory supported host before the preview is considered complete. HTTP/SSE,
pairing persistence, browser assets, shutdown and tunnel documentation must be
tested in GitHub Actions where possible and completed by manual smoke evidence
on both platforms. No GNU-only command or Linux-only socket assumption may be
part of the protocol or installation instructions.

## Phases

1. **Research/ADR:** validate browser/tunnel behavior and threat model; no code.
2. **Loopback observer:** `/meta`, snapshot, SSE replay and bounded in-memory
   projection; no tunnel automation.
3. **Paired PWA:** responsive agents/status/log view, device revocation and
   reconnect/degraded indicators.
4. **Preview hardening:** Linux/macOS evidence, rate/bound/redaction tests,
   security documentation and opt-in release flag.
5. **Optional mutations:** approvals or commands only as separate tasks after
   authoritative digest/revision/idempotency semantics exist.

The research and a narrow observer are expected to fit two to three development
days. Pairing hardening and cross-platform release evidence may require a
separate slice rather than weakening acceptance.

## Acceptance for `CCM-REMOTE-PREVIEW-001`

- this ADR fixes ownership, trust boundary, protocol, non-goals and migration;
- current direct-Codex state is explicitly non-authoritative and read-only;
- snapshot/SSE reconnect, pairing, revocation, bounds and redaction have
  testable acceptance criteria;
- Linux-first and mandatory macOS behavior is stated;
- no raw daemon, public listener or remote mutation is authorized;
- subsequent implementation work is decomposed before code is started.

## Security non-goals for the temporary preview

- no multi-tenant service, OIDC provider, RBAC hierarchy or hosted control
  plane;
- no remote shell/PTY, composer, file download/upload or editor bridge;
- no remote task launch/cancel, approval, permission or sandbox changes;
- no custom TLS, DNS, tunnel, NAT traversal or push-notification service;
- no claim that a tunnel account, source IP or forwarded header alone
  authenticates an application principal;
- no AG-UI, A2A, WebSocket, NATS, Redis or durable orchestration database.

## Migration to `agent-orchestrator`

When the AOR read-only bridge exists, the gateway consumes the same bounded
snapshot/event contract from AOR instead of the TUI projection. Sequence,
cursor, schema negotiation and device-facing envelopes remain stable; AOR
becomes the only source of Task/Run/Approval truth. Pairing/device records may
move to an AOR-owned operator gateway after an explicit data migration.

The temporary gateway must therefore keep provider IDs in binding fields,
avoid inventing Task/Run lifecycle, and isolate direct-mode projection behind a
replaceable source interface. It is removed once the AOR gateway reaches
feature parity; it is never promoted into a second broker.
