# codex-claude-mode

Standalone terminal frontend for the public Codex `app-server` protocol. It
uses an already installed Codex as its backend and does not replace or patch
that installation.

```bash
cargo build --release
target/release/codex-claude-mode \
  --codex "$HOME/.local/bin/codex" \
  --codex-home "$HOME/.codex"
```

Wrapper options are `--codex`, `--codex-home`, `--cwd`, `--thread`, and
`--check-backend`. Other command-line options are forwarded to the installed
Codex before `app-server`; use `--` to force all remaining arguments through.
`--help` prints both this wrapper's options and the selected Codex binary's
original help.

Run a non-interactive compatibility probe first:

```bash
target/release/codex-claude-mode \
  --codex "$HOME/.local/bin/codex" \
  --codex-home "$HOME/.codex" \
  --check-backend
```

The upper, borderless pane shows the selected agent's log. A three-row message
composer sits below it, followed by a horizontal Main/sub-agent bar and the
selected agent's status, elapsed time, token usage, and full ID. `Esc` enters
navigation mode; Left/Right select an agent, PageUp/PageDown/Home/End scroll its
log, and Enter returns to the composer. Up/Down recall message history while
editing. Starting to type in navigation mode enters the composer without
dropping the first character. Mouse selection and wheel scrolling are also
supported. In navigation mode, `i` opens session/agent details including the
session, root, agent and parent IDs, working directory, status, and the exact
rollout/log path reported by Codex; `i`, Esc, or Enter closes the panel. The
wrapper starts Codex with legacy direct-input agents so chats
remain in the selected sub-agent thread instead of being copied through Main.
An older saved parent-owned agent is shown read-only: attempting to message it
explains how to create a new direct agent rather than silently polluting Main.
User messages use a distinct background and a live response timer that freezes
when the first answer text arrives. Structured activity shows reasoning, web and file searches, commands,
tool calls, file changes, and sub-agent actions as they happen. Agent replies
are shown without an `Assistant:` wrapper. Agent markers are `●`
working, `•` idle, `○` closed, and `!` error.

Startup never silently resumes the newest conversation. It first shows `New
session` plus the root sessions found for the current working directory;
Up/Down and Enter create a clean Main or explicitly continue the selected
session. `--thread <id>` bypasses the picker for intentional scripted resumes.
Inside the workspace, `Ctrl-A` selects Main and prepares a natural-language
request for a new sub-agent, `Ctrl-N` starts a clean session, and `Ctrl-R`
reopens the saved-session picker.

Slash input is handled as Codex client commands instead of being sent to the
model as literal text. The standalone frontend currently implements `/new`,
`/clear`, `/resume`, `/skills`, `/status`, `/permissions`, `/agent`, `/subagents`, `/compact`,
`/rename`, `/fork`, `/archive`, `/delete`, `/review`, `/init`, and `/diff`.
`/skills` lists enabled skills for the workspace; `$skill-name` mentions in a
normal message are passed unchanged to Codex's native skill resolver.
`/permissions` lists available profiles and `/permissions <id>` applies one to
subsequent turns through the native `turn/start.permissions` field.
Typing `/` opens a filtered command menu above the composer. Use Up/Down to
select and Enter or Tab to insert a command. `/permissions` opens an allowed
profile picker with Up/Down, Enter to apply, and Esc to cancel. `Ctrl-U` clears
the current composer line without corrupting a draft saved behind an approval.
Left/Right in the normal editing composer move the text cursor. Press `Esc` to
enter navigation mode before using Left/Right to select the previous/next agent;
the composer draft is preserved.
Tab also completes executable names from `PATH` and filesystem paths relative
to the workspace. Multiple matches open a selectable completion popup; no shell
startup files or user-entered commands are executed while completing.
Multiline terminal pastes are collapsed to `[Pasted text #N +K lines]` markers
in the composer and expanded back to their original contents when submitted.
PageUp/PageDown and the mouse wheel continue scrolling the log while editing.

From navigation mode, `t` opens the selected agent's project tree. Use arrows
or `h/j/k/l` to navigate, `g/G` for the first/last entry, and Enter or `l` to
expand a directory or view a file. The built-in viewer shows line numbers and
syntax highlighting. Press `e` for `$VISUAL`/`$EDITOR` (falling back to Vim),
`v` for VS Code, or `c` for Cursor. `q`, Esc, or `t` closes the browser.

Agent logs are prefetched once and switching uses the in-memory cache without a
new `thread/read`. Agents stay ordered by creation time. When an older Codex
backend requires a sub-agent message to travel through Main, the transport turn
is hidden while the selected sub-agent's own answer appears in its log.

Interactive command, file-change, and permission approvals are shown in the log
pane with explicit allow/deny scope. File approvals include the paths and diff
received in the matching `item/started` event; keyboard or mouse scrolling keeps
long patches reviewable. If the backend supplies no change details, the prompt
says so instead of presenting an opaque item ID as sufficient evidence. Codex
user-input questions and MCP elicitations are handled there as well. Unknown or
overlapping requests fail closed; the frontend never silently accepts an
approval. Press `Ctrl-C` to interrupt the selected active turn. Press `Ctrl-D`
once to leave editing and again to quit; any other key cancels the pending exit.
`Ctrl-Q` remains the immediate quit shortcut.
File-change approvals keep the question and decisions in a highlighted fixed
panel. `Ctrl-A` opens the full patch in a separate `P A T C H` pager matching
Codex navigation (arrows, j/k, PageUp/PageDown, Ctrl-U/Ctrl-D, Home/End); q or
Ctrl-C returns to the still-pending approval. Reasoning/Thinking is shown only
as the live header status, not repeated in the log.

The local frontend and the installed Codex are separate processes. Codex
credentials remain in the local `CODEX_HOME`; this project neither imports nor
reuses them for its own user or host authentication.

See [ARCHITECTURE.md](ARCHITECTURE.md) for accepted architecture and future
remote constraints, [MULTI_AGENT_SPEC.md](MULTI_AGENT_SPEC.md) for requirements,
[ROADMAP.md](ROADMAP.md) for phases and gates, and [TODO.md](TODO.md) for atomic
task status. [PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md) records the current
Linux/macOS evidence, gaps, required CI matrix and manual smoke checks.
