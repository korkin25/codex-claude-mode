# Linux and macOS platform support

Evidence and required validation for `SHARED-PLATFORM-001`, captured on
2026-08-12. This matrix describes the current direct-Codex client. Planned
orchestrator functionality is included only to define its future platform gate.

## Evidence vocabulary

- **Tested** — exercised on the named OS/architecture by recorded automated or
  manual evidence.
- **Expected** — uses portable APIs and is expected to work, but has no evidence
  on that OS/architecture.
- **Degraded** — works with a documented loss of capability or fidelity.
- **Unsupported** — deliberately outside the supported contract.
- **Unverified** — implementation exists, but the available evidence cannot
  establish its runtime behavior.

`Supported` is a release-level claim: it requires all mandatory jobs and smoke
checks below. Source compatibility or a successful cross-build alone is not
support evidence.

## Current capability matrix

| Capability | Linux x86_64 | Linux ARM64 | macOS x86_64 | macOS ARM64 | Current boundary or gap |
|---|---|---|---|---|---|
| Rust build, unit tests and Clippy | **Tested** with Rust 1.95.0: 72 tests | **Expected** | **Expected** | **Expected** | `CCM-CI-001` adds mandatory Linux x86_64 and macOS ARM64 jobs; macOS remains expected until that job succeeds. |
| Release packaging | **Tested** only as historical `0.4.7` artifact; not reproducible from a tag | **Unverified** | **Unverified** | **Unverified** | Checksums exist only for Linux x86_64. No signing or provenance. |
| Direct Codex process and app-server JSONL | **Unverified** end to end | **Unverified** | **Unverified** | **Unverified** | Unit tests validate parsing/projections, not a supported Codex-version runtime matrix. Codex CLI availability is an external prerequisite. |
| TUI raw mode, alternate screen, keyboard and mouse | **Unverified** manually | **Unverified** | **Expected** | **Expected** | Crossterm is portable, but panic/signal/child-exit restoration is not guarded or tested. Terminal behavior varies by emulator. |
| Session picker, agent tree, logs, composer and approvals | **Tested** by unit/render tests | **Expected** | **Expected** | **Expected** | No macOS snapshots or live app-server smoke evidence. |
| Project tree and built-in file viewer | **Tested** by Linux unit tests | **Expected** | **Expected** | **Expected** | UTF-8 text only; files over 2 MiB show an error. Dotfiles, `target` and `node_modules` are intentionally hidden. Syntax highlighting is a small extension/keyword highlighter, not a parser. |
| Terminal editor via `VISUAL`/`EDITOR`, fallback `vim` | **Tested** for argv construction/path confinement | **Expected** | **Expected** | **Expected** | Actual terminal suspend/resume and editor availability are unverified. Environment value is parsed without a shell. |
| VS Code/Cursor launch | **Tested** for argv construction | **Expected** | **Unverified** | **Unverified** | Requires `code`/`cursor` on `PATH`; no explicit macOS `open -a` fallback or application discovery. |
| Codex version display and `U` updater | **Tested** for parsing/UI confirmation | **Expected** | **Expected** | **Expected** | Real `codex update`, failure recovery and installation-method behavior are unverified on every OS. |
| Shell/path completion | **Tested** on Linux | **Expected** | **Expected** | **Expected** | Executable-bit logic is Unix-specific as intended. It does not invoke bash/zsh startup files and is not full shell grammar completion. |
| UTF-8 paths and symlink confinement | **Tested** on Linux for ordinary UTF-8 paths and a Unix symlink escape | **Expected** | **Expected** | **Expected** | Canonicalization is portable, but case-insensitive filesystems and macOS Unicode normalization are untested. |
| Non-UTF-8 OS paths | **Degraded** | **Degraded** | **Degraded** | **Degraded** | Several protocol/display/editor boundaries use lossy string conversion. Such paths must not be treated as stable identity until fixed. |
| Cancellation, signals and process groups | **Degraded** | **Degraded** | **Degraded** | **Degraded** | Backend drop calls `Child::kill` only. No process-group ownership, graceful escalation, signal restoration test or guarantee that grandchildren stop. |
| Future JSONL stdio bridge | **Expected, planned** | **Expected, planned** | **Expected, planned** | **Expected, planned** | No bridge is implemented. Versioned envelopes and golden fixtures must be OS-neutral. |
| Future Unix-socket IPC | **Unsupported today** | **Unsupported today** | **Unsupported today** | **Unsupported today** | Must probe path-length, ownership/mode and stale-socket behavior separately on Linux and macOS. |

Windows is unsupported for the MVP. That exclusion does not permit platform
native handles or lossy paths to enter the future wire protocol.

## Source portability audit

The current crate has no unconditional Linux-only Rust API, `/proc` access,
hard-coded `/bin/bash`, GNU utility invocation or shell interpolation. The
following assumptions still prevent a first-class macOS support claim:

| Finding | Source | Impact | Required disposition |
|---|---|---|---|
| Backend termination uses `Child::kill` and does not own a process group. | `src/backend.rs` (`Drop for Backend`) | App-server descendants may survive; graceful cancellation and signal semantics are not distinguished. | Add a platform process-supervisor abstraction and Linux/macOS integration tests before claiming lifecycle parity. |
| TUI cleanup is performed on normal returns and around terminal editors, without an RAII/panic/signal restoration guard. | `src/main.rs` (`run_tui`, `run_terminal_editor`) | A panic, termination signal or some early failures can leave raw/alternate-screen state behind. | Add a terminal guard and PTY tests on both OSes. |
| CWD values cross JSON through `Path::to_string_lossy`. | `src/main.rs` (`thread/start`, skills, permissions and list params) | Distinct non-UTF-8 paths can alias or target the wrong workspace. | Use a lossless/typed protocol representation or fail explicitly before request submission. |
| Terminal-editor path is converted with `to_string_lossy`; tree names and titles are also lossy. | `src/editor.rs` (`terminal_command`), `src/project_tree.rs` | Editor targeting can be wrong; display strings cannot be used as identity. | Keep `OsString` for argv and separate native identity from escaped display text. |
| `$HOME` is used directly and the default Codex home is a debug-specific `tmp/codex-agent-picker-test-home`. | `src/main.rs` (`default_codex_home`) | Ignores platform directory conventions and is surprising on both OSes. | Make normal Codex home semantics explicit; isolate test-home behavior behind configuration/tests. |
| GUI editors are spawned only as `code` or `cursor` from `PATH`. | `src/editor.rs` (`command_for`) | Common macOS application installs may not expose CLI shims. | Capability-probe CLI shims; add an explicit, argv-only `open -a` fallback if approved. |
| VS Code/Cursor targets are assembled from `Path::display`, and completion drops names that are not UTF-8. | `src/editor.rs` (`goto_command`), `src/shell_completion.rs` (`command_candidates`, `filesystem_candidates`) | A path can disappear from completion or point at a lossy alias. | Preserve native path values locally and fail explicitly at unavoidable text protocol boundaries. |
| A Vim-style `+line` argument is appended to every configured terminal editor. | `src/editor.rs` (`terminal_command`) | `nano`, Emacs and other valid `VISUAL`/`EDITOR` programs may interpret it differently. | Use location templates for known editors; otherwise pass only the native file path. |
| Project viewer uses `read_to_string`. | `src/project_tree.rs` (`Viewer::load`) | Non-UTF-8/binary files cannot be viewed, though failure is safe. | Report typed `binary/unsupported encoding`; never silently decode lossy content. |
| Project traversal silently omits read/metadata failures and does not identify symlinks separately. | `src/project_tree.rs` (`append_children`) | Inaccessible entries and macOS aliases/symlinks appear missing or confusing. | Render explicit inaccessible/symlink states while retaining canonical editor confinement. |
| Unix executable detection is `mode & 0o111`; non-Unix builds treat every file as executable. | `src/shell_completion.rs` (`is_executable`) | Correct for Linux/macOS, but documents why Windows is not supported. | Keep Unix tests on both mandatory OSes; revise before adding Windows. |
| Symlink escape coverage is gated by `cfg(unix)`. | `src/editor_tests.rs` | Covers the API family but has only Linux execution evidence. | Run the same test on macOS CI and add case/Unicode volume fixtures. |
| Tests create PID-named directories in the shared temporary directory. | `src/editor_tests.rs`, `src/project_tree_tests.rs`, `src/shell_completion_tests.rs`, `src/ui_tests.rs` | Concurrent test processes can collide and cleanup failures can leave fixtures. | Introduce a dependency-free unique test-directory helper or a standard tempfile crate in a separate change. |

## Required CI jobs

The following jobs are the minimum gate for claiming source support. The first
two are implemented by `.github/workflows/ci.yml`; a committed workflow is not
runtime evidence until its named job succeeds.

1. `linux-x86_64-rust` on a pinned Ubuntu runner and Rust 1.95.0:
   `cargo fmt --all -- --check`, `cargo test`, and
   `cargo clippy --all-targets -- -D warnings`.
2. `macos-arm64-rust` on the `macos-14` runner, with an explicit `uname -m =
   arm64` assertion, the same commands and toolchain. A runner-label change
   fails visibly instead of being misreported as ARM64 evidence.
3. `macos-x86_64-build` on Intel macOS, or a native Intel release runner if the
   project claims runtime support. A cross-build without execution is build
   evidence only.
4. `linux-arm64-build-test` on a native or emulated ARM64 runner. Emulation must
   be labelled and cannot replace the direct Codex/manual runtime smoke.
5. `platform-path-tests` on Linux and macOS: UTF-8 spaces, symlink escape,
   filesystem case behavior, decomposed/precomposed Unicode on the actual
   volume, and an explicit typed result for non-UTF-8 where the OS permits it.
6. `platform-pty-tests` on Linux and macOS: normal exit, error, panic and signal
   restore raw mode/alternate screen; terminal-editor suspend/resume works.
7. `direct-codex-contract` on Linux x86_64 and macOS ARM64 against each declared
   supported Codex version: initialize, session list/start/resume, turn,
   streaming items, approval, interruption, permission update and shutdown.
   Credentials and billable inference are not required for protocol fixtures;
   any live-provider smoke is a separately protected job.
8. `package-linux-{x86_64,aarch64}` and
   `package-macos-{x86_64,aarch64}`: build from an immutable commit/tag, run
   `--version`, verify architecture, produce checksum and provenance. macOS
   signing/notarization may remain a separately reported release capability.

Packaging must use a tracked portable script or workflow, avoid GNU-only
utility flags, and record version, commit, target and SHA-256 in a manifest.

Jobs must publish OS, architecture, Rust version, Codex version and terminal
fixture as evidence. A skipped mandatory job makes the capability `unverified`,
not green.

## Manual smoke checklist

Run this checklist on Linux x86_64 and macOS ARM64 before the first supported
release; add Intel/ARM variants when releasing those artifacts.

- Launch `--help`, `--version` and `--check-backend` with paths containing
  spaces and Unicode.
- Start a new session, resume an explicit session and switch between Main and
  a live sub-agent without mixed logs or lost drafts.
- Type, edit and paste Unicode/multiline/long input; verify cursor, whitespace,
  viewport and log scrolling in editing and navigation modes.
- Exercise slash completion, path completion and permission selection; confirm
  an incoming approval takes focus and the picker/draft returns afterward.
- Approve and reject command/file changes, inspect a long patch, interrupt a
  turn and verify no backend descendant remains.
- Open the project tree, a UTF-8 source file, a large/invalid file, and attempt
  a symlink escape. Confirm the latter is rejected.
- Launch the configured terminal editor, VS Code and Cursor where advertised;
  verify exact file/line and terminal restoration after success, failure and
  cancellation.
- Open session info and test updater confirmation. Run the real updater only in
  an isolated disposable Codex installation and verify failure reporting.
- Send `SIGINT`, `SIGTERM` and force an application panic in the test harness;
  verify terminal restoration and process cleanup.
- Validate the packaged binary on a clean machine with no development checkout,
  record checksum/architecture, and confirm missing optional editors produce a
  useful error rather than a crash.

## Gate decision

This document completes the matrix-and-gaps planning acceptance of
`SHARED-PLATFORM-001`; it does not establish macOS support or close `G0`.
Platform support becomes a release claim only after the mandatory jobs and
smoke evidence exist and the degraded lifecycle/path items are either fixed or
explicitly excluded from the release contract.
