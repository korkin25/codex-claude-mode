# Changelog

## 0.4.10 — 2026-08-13

- Fixed the checksum path stored inside release archives so `SHA256SUMS` can be
  verified directly from the extracted package directory.
- Added a release-workflow check that extracts every platform archive and
  verifies its internal binary checksum before upload.

## 0.4.9 — 2026-08-12

- Fixed multiline terminal paste so pasted line breaks are collapsed into one
  composer placeholder and submitted as one message.
- Added clipboard image attachments with `Alt-I` on Linux and macOS, compact
  composer placeholders and native Codex `localImage` turn input.
- Added `$skill` discovery and completion with exact selected skill forwarding,
  including correct handling of duplicate skill names.
- Preserved image attachments in resumed session history and bracketed-paste
  behavior when returning from a terminal editor.
- Hardened clipboard capture with bounded background workers, private temporary
  storage, stale cleanup and process cancellation on exit.

## 0.4.8 — 2026-08-12

- Added direct sub-agent navigation with isolated logs, explicit session
  selection and hidden closed agents.
- Added slash-command and permission-profile pickers, per-agent permission
  state, visible/default approval choices and a full patch pager.
- Added Unicode-aware composer editing, long-input viewport, multiline-paste
  placeholders, shell/path completion and log scrolling in either input mode.
- Added session diagnostics, Codex version/update controls, a project tree,
  syntax-highlighted file viewer and Vim/VS Code/Cursor launch actions.
- Added pinned Rust 1.95 CI and release builds for Linux x86_64 and macOS ARM64.

The multi-provider orchestrator, durable agent bus and remote integrations
described in the roadmap remain planned and are not part of this release.
