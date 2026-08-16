# Roadmap

This project is a focused terminal workspace for the installed Codex CLI. The
roadmap covers only that scope: making the existing direct-Codex client more
useful. Atomic status lives in [TODO.md](TODO.md).

## Current state

The client is a single process. It spawns `codex app-server`, renders the
session and sub-agent tree, and dies together with its terminal. Everything
below follows from wanting to keep sessions reachable when that terminal is
gone.

## Core delivery lane

1. **Separate state from rendering.** Move app-server ownership and the
   projection into a headless `serve` process with a local Unix socket. The TUI
   becomes one of its clients and keeps a `--direct` fallback to today's
   behavior.
2. **Add a scriptable client.** `codex-claude-mode ctl` speaks the same socket
   with `--json` output, so sessions can be listed, created, read, answered and
   approved without the TUI.
3. **Ship an agent-framework skill.** A skill under `skills/` drives `ctl`, so
   an agent framework already running on the host can operate sessions. That is
   what makes the workspace reachable from a phone without this project
   implementing any remote transport, listener or account system.
The dependency graph is explicit and permits independent work after the first
boundary is verified:

```text
CCM-SERVE-001
    ├── CCM-SERVE-002
    └── CCM-CTL-001 ──────── CCM-SKILL-001
```

`CCM-SERVE-002` and `CCM-CTL-001` may proceed in parallel after
`CCM-SERVE-001` is verified. The framework skill depends on the scriptable
`ctl` interface, not on the sibling TUI migration.

## Independent lanes

- **Direct compatibility:** `CCM-DIRECT-001` can proceed without `serve`; it
  preserves and verifies today's direct-Codex fallback.
- **Optional daemon transport:** `CCM-DAEMON-001` is an optional extension after
  `CCM-SERVE-001`. It is blocked because a measured Codex 0.147.0 probe did not
  provide the required proxy relay; a newer upstream Codex version must be
  re-probed before implementation. This lane is not on the core delivery path
  and cannot block it.

Machine-readable status and evidence rules live in
[`delivery/capabilities.json`](delivery/capabilities.json). A declaration is
not implementation evidence: consumers must require a `verified` entry bound to
an exact merge SHA, successful CI on that SHA and a measured content digest.

## Non-goals

- No network listener, TLS termination, tunnel management or account system.
- No second scheduler, broker or authoritative task database.
- No patching, vendoring or replacing the installed Codex CLI.
- No screen scraping or key injection into terminals as a source of state.

Linux x86_64 and macOS ARM64 are intended first-class targets. Support claims
remain capability- and evidence-gated; see
[PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md) for the measured matrix and gaps.
