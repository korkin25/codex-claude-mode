# codex-claude-mode

Standalone terminal frontend for the public Codex `app-server` protocol. It
uses an already installed Codex as its backend and does not replace or patch
that installation.

```bash
cargo build --release
target/release/codex-claude-mode \
  --codex /home/kk573/.local/bin/codex \
  --codex-home /home/kk573/tmp/codex-agent-picker-test-home
```

Run a non-interactive compatibility probe first:

```bash
target/release/codex-claude-mode \
  --codex /home/kk573/.local/bin/codex \
  --codex-home /home/kk573/tmp/codex-agent-picker-test-home \
  --check-backend
```

The left pane contains Main and its sub-agents. `Esc` enters navigation mode,
arrow keys select an agent, PageUp/PageDown/Home/End scroll its log, and Enter
returns to the message composer. Mouse selection and wheel scrolling are also
supported. Messages for sub-agents that reject direct turns are routed through
Main, matching the app-server capability contract.

Interactive command, file-change, and permission approvals are shown in the log
pane with explicit allow/deny keys. Codex user-input questions and MCP
elicitations are handled there as well. Unknown or overlapping requests fail
closed; the frontend never silently accepts an approval. Press `Ctrl-C` to
interrupt the selected active turn and `Ctrl-Q` to quit from any mode.

The local frontend and the installed Codex are separate processes. Codex
credentials remain in the local `CODEX_HOME`; this project neither imports nor
reuses them for its own user or host authentication.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the remote-access design and
[TODO.md](TODO.md) for the staged implementation plan.
