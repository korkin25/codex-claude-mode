# Roadmap

This project is a focused terminal workspace for the installed Codex CLI. The
roadmap covers only that scope: making the existing direct-Codex client more
useful. Atomic status lives in [TODO.md](TODO.md).

## Current state

The client is a single process. It spawns `codex app-server`, renders the
session and sub-agent tree, and dies together with its terminal. Everything
below follows from wanting to keep sessions reachable when that terminal is
gone.

## Direction

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
4. **Optionally use the Codex daemon.** Connect `serve` to
   `codex app-server daemon` so agents survive a `serve` restart.

## Non-goals

- No network listener, TLS termination, tunnel management or account system.
- No second scheduler, broker or authoritative task database.
- No patching, vendoring or replacing the installed Codex CLI.
- No screen scraping or key injection into terminals as a source of state.

Linux x86_64 and macOS ARM64 are both supported; see
[PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md).
