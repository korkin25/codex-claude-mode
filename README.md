# codex-claude-mode

**A focused terminal workspace for running Codex with multiple agents.**

`codex-claude-mode` is a standalone TUI for the public Codex `app-server`
protocol. It gives developers one place to start or resume a Codex session,
switch between Main and sub-agents, follow each agent's work, review approvals,
and browse the project without losing context.

It uses your existing Codex installation, authentication, configuration, and
session files. It does not replace or patch the Codex CLI.

## What it does

- Keeps Main and sub-agent conversations in separate, directly selectable logs.
- Shows live reasoning status, tool activity, commands, file changes, token use,
  elapsed time, and agent state.
- Lets you create, resume, rename, fork, archive, delete, review, and compact
  sessions from the terminal.
- Handles command, permission, file-change, user-input, and MCP approval prompts;
  file changes can be reviewed in a full patch viewer before approval.
- Includes slash-command and permission-profile pickers, input history,
  collapsed multiline paste, clipboard image attachments, and shell/path
  completion.
- Includes a project tree, syntax-highlighted file viewer, and shortcuts to open
  files in Vim, VS Code, or Cursor.

It is aimed at developers who already use Codex and want a keyboard-first view
of multi-agent work, especially when several agents are active at once.

> **Experimental community software.** This project depends on the experimental
> Codex `app-server` protocol and may need updates when that protocol changes. It
> is not affiliated with, endorsed by, or supported by OpenAI, Anthropic,
> Google, xAI, Microsoft, or Anysphere.

## Quick start

You need an installed and authenticated `codex` CLI. Confirm that it works:

```bash
codex --version
```

Release binaries are available for Linux x86_64 and macOS Apple Silicon. They
install into `~/.local/bin`; `sudo` is not required.

### Linux x86_64

```bash
version="0.4.9"
platform="linux-x86_64"
base="https://github.com/korkin25/codex-claude-mode/releases/download/v${version}"
curl -fLO "${base}/codex-claude-mode-${version}-${platform}.tar.gz"
curl -fLO "${base}/SHA256SUMS"
grep "codex-claude-mode-${version}-${platform}.tar.gz" SHA256SUMS | sha256sum -c -
tar -xzf "codex-claude-mode-${version}-${platform}.tar.gz"
mkdir -p "$HOME/.local/bin"
cp "codex-claude-mode-${version}-${platform}/codex-claude-mode" "$HOME/.local/bin/"
chmod 755 "$HOME/.local/bin/codex-claude-mode"
"$HOME/.local/bin/codex-claude-mode" --version
"$HOME/.local/bin/codex-claude-mode" --check-backend
"$HOME/.local/bin/codex-claude-mode"
```

### macOS Apple Silicon

```bash
version="0.4.9"
platform="macos-arm64"
base="https://github.com/korkin25/codex-claude-mode/releases/download/v${version}"
curl -fLO "${base}/codex-claude-mode-${version}-${platform}.tar.gz"
curl -fLO "${base}/SHA256SUMS"
grep "codex-claude-mode-${version}-${platform}.tar.gz" SHA256SUMS | shasum -a 256 -c -
tar -xzf "codex-claude-mode-${version}-${platform}.tar.gz"
mkdir -p "$HOME/.local/bin"
cp "codex-claude-mode-${version}-${platform}/codex-claude-mode" "$HOME/.local/bin/"
chmod 755 "$HOME/.local/bin/codex-claude-mode"
"$HOME/.local/bin/codex-claude-mode" --version
"$HOME/.local/bin/codex-claude-mode" --check-backend
"$HOME/.local/bin/codex-claude-mode"
```

If `~/.local/bin` is not on your `PATH`, add this to `~/.zshrc` (macOS) or
your shell's equivalent startup file:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

The macOS archive is currently unsigned and not notarized. After verifying its
checksum, if Gatekeeper quarantines the binary, remove quarantine from that
binary only:

```bash
xattr -d com.apple.quarantine "$HOME/.local/bin/codex-claude-mode"
```

## Using the workspace

At startup, choose **New session** or explicitly resume a saved root session for
the current directory. `Ctrl-A` selects Main and prepares a request for a new
sub-agent; use the agent bar to move between its separate conversations.

Type `/` to browse supported client commands, including `/new`, `/resume`,
`/permissions`, `/agent`, `/subagents`, `/review`, `/diff`, and `/skills`.

Run `codex-claude-mode --help` for wrapper options and the selected Codex
backend's original options. Useful wrapper options include `--codex`,
`--codex-home`, `--cwd`, `--thread`, and `--check-backend`.

## Keyboard shortcuts

Shortcuts are mode-specific. In particular, arrow keys move the text cursor in
**Editing** and switch agents only in **Navigation**.

### Navigation and logs

| Mode | Keys | Action |
| --- | --- | --- |
| Any workspace mode | `Ctrl-A` / `Ctrl-N` / `Ctrl-R` | Prepare a sub-agent request / start a clean session / open the session picker |
| Any workspace mode | `Ctrl-C` | Interrupt the selected active turn |
| Any workspace mode | `Ctrl-D` twice / `Ctrl-Q` | Confirmed quit / immediate quit |
| Navigation | `Left` or `Up` / `Right` or `Down` | Select the previous / next agent |
| Navigation | `PageUp` / `PageDown`, `Home` / `End` | Scroll the selected log / jump to its start or end |
| Navigation | `Enter` | Enter Editing mode |
| Navigation | Any unmodified character | Enter Editing and keep the typed character |
| Navigation | `i` | Open session and agent details |

### Editing and composer

| Mode | Keys | Action |
| --- | --- | --- |
| Editing | `Esc` | Enter Navigation mode |
| Editing | `Left` / `Right`, `Home` / `End` | Move the text cursor / jump to input start or end |
| Editing | `Up` / `Down` | Recall older / newer input |
| Editing | `PageUp` / `PageDown` | Scroll the log without leaving the composer |
| Editing | `Enter` | Submit non-empty input |
| Editing | `Ctrl-U` | Clear the current input |
| Editing | `Alt-I` | Attach a PNG/JPEG image from the clipboard |
| Editing | `Tab` | Complete a slash command, executable, or workspace path |
| Completion menu | `Up` / `Down`, `Enter` or `Tab`, `Esc` | Select, apply, or close completion |
| Skill menu | Type `$`, then `Up` / `Down`, `Enter` or `Tab`, `Esc` | Filter, insert, or close enabled skill mentions |

### Approvals and permissions

| Context | Keys | Action |
| --- | --- | --- |
| Approval or permission prompt | `Up` / `Down` or `k` / `j`, `Enter` | Select and confirm an offered decision |
| Approval prompt | `y` / `a` / `n` / `x` | Approve once / approve for session / decline / cancel, when offered |
| Permission prompt | `y` / `a` / `n` / `x` | Allow for turn / allow for session / deny for turn / deny and interrupt |
| File-change approval | `Ctrl-A` | Open the full patch viewer |
| Any active prompt | `PageUp` / `PageDown`, `Home` / `End` | Scroll the log behind the fixed prompt |
| Any active prompt | `Ctrl-C` | Cancel the prompt and interrupt its turn |
| Permission-profile picker | Arrows, `Enter`, `Esc` | Select, apply, or close the profile picker |

### Project tree, file viewer, and diff

| Context | Keys | Action |
| --- | --- | --- |
| Navigation | `t` | Open the selected agent's project tree |
| Project tree | Arrows or `h` / `j` / `k` / `l`, `Enter` | Navigate, collapse, expand, or open |
| Project tree | `g` / `G` | Jump to the first / last entry |
| Project tree | `e` / `v` / `c` | Open the selected file in the terminal editor / VS Code / Cursor |
| Project tree | `q`, `Esc`, or `t` | Close the tree |
| File viewer | `Up` / `Down` or `k` / `j`, `PageUp` / `PageDown`, `g` / `G` | Scroll by line, page, or to the start/end |
| File viewer | `Left` or `h`, `q`, or `Esc` | Return to the project tree |
| File viewer | `e` / `v` / `c` | Open the file in the terminal editor / VS Code / Cursor |
| Patch viewer | `Up` / `Down` or `k` / `j`, `PageUp` / `PageDown`, `Ctrl-U` / `Ctrl-D`, `Home` / `End` | Scroll the diff |
| Patch viewer | `q` or `Ctrl-C` | Return to the pending approval |
| Editing | `/diff` | Request the current workspace diff from Codex |

## Build from source

Building requires Rust 1.95:

```bash
git clone https://github.com/korkin25/codex-claude-mode.git
cd codex-claude-mode || exit
git checkout v0.4.9
cargo build --locked --release
target/release/codex-claude-mode --check-backend
target/release/codex-claude-mode
```

## Current limitations

- Prebuilt binaries are limited to Linux x86_64 and macOS ARM64. Intel macOS is
  unverified and has no release artifact.
- The frontend and Codex run as separate local processes. Codex credentials stay
  in `CODEX_HOME`; this project does not provide separate authentication.
- Older saved parent-owned agents are read-only because direct replies could be
  copied into Main. Create a new direct sub-agent instead.
- Clipboard images require `wl-paste` (preferred) or `xclip` on Linux, and the
  optional `pngpaste` utility on macOS. Text paste uses terminal bracketed-paste
  support and multiline text is shown as one compact placeholder.
- Multi-provider orchestration, a durable agent bus, and remote integrations are
  roadmap items, not features in v0.4.9.

To update, repeat the verified release download with a newer version. To
uninstall:

```bash
rm "$HOME/.local/bin/codex-claude-mode"
```

## More information

- [Architecture and security boundaries](ARCHITECTURE.md)
- [Multi-agent behavior specification](MULTI_AGENT_SPEC.md)
- [Platform support and smoke checks](PLATFORM_SUPPORT.md)
- [Roadmap](ROADMAP.md)
- [Task status](TODO.md)
- [Changelog](CHANGELOG.md)
- [Security policy](SECURITY.md)

Licensed under the [Apache License 2.0](LICENSE).
