# Direct-Codex baseline inventory

Evidence for `CCM-BASE-003`, captured on 2026-08-12. This document separates
the packaged Linux `0.4.7` binary from the source tree that continued changing
after that package was built.

## Evidence boundary

| Evidence | Result |
|---|---|
| Packaged binary | `dist/codex-claude-mode-0.4.7-linux-x86_64/codex-claude-mode`, version output `0.4.7`, SHA-256 `c37f89adacd6a945a3b4df708112b349622d435161fa790c1433afd1ebd0f06c` |
| Packaged archive | `dist/codex-claude-mode-0.4.7-linux-x86_64.tar.gz`, SHA-256 `ded0d91f4db15043060f8d5e6fd6f675a3e355a8f39c05ff525834391409edca` |
| Package timestamp | binary `2026-08-12 08:23:59 +0800`; archive `08:23:59 +0800` |
| Current source version | `Cargo.toml` says `0.4.7`, but the worktree is dirty and is not the packaged source identity |
| Current source tests | 72 passed with Rust 1.95.0 on Linux on 2026-08-12 |
| Reproducibility | not established: the package has no corresponding commit/tag and current `src/ui.rs` and `src/ui_tests.rs` were modified after the package timestamp |
| Platform evidence | Linux x86_64 only; macOS and ARM64 remain `SHARED-PLATFORM-001` |

The package is therefore a historical binary snapshot, not a releasable
baseline commit. Passing tests below verify the current dirty tree and must not
be retroactively treated as proof of the packaged binary.

## User-facing inventory

`Shipped` means present in the last packaged `0.4.7` snapshot based on the
package contents, pre-package source timestamps and packaged documentation.
`Tested after package` means a fix landed in the still-dirty tree after the
binary timestamp and is covered by the current test run. `Unverified` means the
code exists but lacks the required end-to-end or platform evidence. `Planned`
means no implementation is claimed.

| State | User-facing behavior | Implementation | Test or release evidence | Follow-up |
|---|---|---|---|---|
| Shipped | Explicit session picker, Main/sub-agent tree, stable ordering and hidden closed agents | `src/session.rs`, `src/model.rs`, `src/ui.rs`, `src/main.rs` | packaged `0.4.7`; `session_tests.rs`, `model_tests.rs`, `ui_tests.rs::{session_picker_defaults_to_new_and_can_continue_existing_session,tree_keeps_main_above_nested_agents,tree_hides_closed_subagents_but_keeps_idle_ones}` pass in current tree | direct compatibility gaps belong to `CCM-DIRECT-001` |
| Shipped | Direct selected-agent logs and direct-input sub-agent behavior without leaking Main transport turns | `src/model.rs`, `src/main.rs`, `src/backend.rs` | packaged `0.4.7`; `model_tests.rs::{main_history_hides_subagent_transport_turns,parses_agent_metadata_and_history}` pass | live reconnect/resume coverage: `CCM-DIRECT-001` |
| Shipped | Slash command parsing, popup and original Codex CLI option forwarding/combined help | `src/command.rs`, `src/main.rs`, `src/ui.rs` | packaged binary `--help` shows wrapper and installed Codex options; `command_tests.rs`, `main_tests.rs::splits_wrapper_and_codex_options`, `ui_tests.rs::slash_input_opens_filterable_command_menu_and_inserts_selection` pass | runtime schema probing: `CCM-DIRECT-001` |
| Shipped | `$skill` text passes through and `/skills` is handled as a client command | `src/command.rs`, `src/main.rs` | documented in packaged `README.md`; slash parser tests cover command dispatch boundary | native skill resolution requires live backend compatibility evidence under `CCM-DIRECT-001` |
| Shipped | Permission picker, selected-agent permission indicator, per-thread update and subsequent-turn propagation | `src/main.rs`, `src/ui.rs` | packaged `0.4.7`; `main_tests.rs::{permission_selection_updates_the_existing_backend_thread,next_turn_keeps_the_selected_permission_profile}` and permission UI tests pass | live provider semantics remain unverified below |
| Shipped | Approval choices have a highlighted default, Enter/navigation, fail-closed unknown handling and preserved draft/picker | `src/prompt.rs`, `src/ui.rs` | packaged `0.4.7`; `prompt_tests.rs::{approval_default_and_navigation_resolve_the_highlighted_available_choice,approval_default_never_selects_an_unavailable_decision,permission_request_defaults_to_allow_once_and_can_select_deny}` and approval UI tests pass | live backend matrix: `CCM-DIRECT-001` |
| Shipped | File approval keeps decisions visible and opens a scrollable patch pager | `src/prompt.rs`, `src/ui.rs` | packaged `0.4.7`; `prompt_tests.rs::file_change_approval_shows_paths_diff_reason_and_decision_scope`, `ui_tests.rs::long_file_approval_keeps_decisions_visible_and_opens_patch_pager` pass | exact cross-version Codex parity: `CCM-DIRECT-001` |
| Shipped | Info panel, Codex version comparison and confirmed `U` update action | `src/version.rs`, `src/main.rs`, `src/ui.rs` | packaged `0.4.7`; `version_tests.rs`, `ui_tests.rs::{navigation_i_opens_agent_info_without_editing,info_overlay_confirms_codex_update_before_returning_action}` pass | actual updater execution is unverified below |
| Shipped | Shell/path completion, multiline-paste placeholders, Unicode cursor editing and long-composer viewport | `src/shell_completion.rs`, `src/ui.rs` | packaged `0.4.7`; `shell_completion_tests.rs` and cursor/paste/viewport tests in `ui_tests.rs` pass | shell/platform parity: `SHARED-PLATFORM-001` |
| Shipped | Project tree, syntax-highlighted file viewer and Vim/VS Code/Cursor launch actions | `src/project_tree.rs`, `src/editor.rs`, `src/main.rs` | sources predate package timestamp; packaged README documents it; `project_tree_tests.rs` and `editor_tests.rs` pass | actual editors and macOS commands are unverified below |
| Tested after package | Spaces typed at wrapped composer boundaries are preserved and reflected immediately | `src/ui.rs` | `ui_tests.rs::{composer_preserves_spaces_and_moves_cursor_immediately,composer_preserves_space_at_wrapped_boundary}` pass; both source/test files postdate packaged binary | include in the next versioned, committed release slice |
| Tested after package | Long Main logs scroll from follow-tail in navigation/editing and via mouse | `src/ui.rs` | `ui_tests.rs::{page_up_from_bottom_scrolls_long_main_log_on_first_press,mouse_scroll_up_from_bottom_scrolls_main_log_and_end_keeps_following,editing_page_keys_scroll_log_without_changing_draft_or_mode}` pass; `ui.rs`/tests postdate package | include in the next versioned, committed release slice |
| Unverified | Real installed app-server lifecycle across supported Codex versions: start, reconnect, resume, pagination and unknown runtime schema | `src/backend.rs`, `src/main.rs` | unit tests do not exercise the installed app-server matrix | `CCM-DIRECT-001` |
| Unverified | `/permissions` changes effective sandbox/permission behavior for an already running live sub-agent, not only request payloads | `src/main.rs` | JSON parameter tests pass, but no live behavioral integration test is recorded | `CCM-DIRECT-001`; platform behavior also `SHARED-PLATFORM-001` |
| Unverified | `U` completes a real Codex upgrade and refreshes version state safely | `src/version.rs`, `src/main.rs` | parsing/confirmation tests only; no isolated updater integration evidence | decompose under `CCM-DIRECT-001` before release claim |
| Unverified | Vim, VS Code and Cursor launch successfully and safely on Linux and macOS | `src/editor.rs`, `src/project_tree.rs` | argument construction and path-confinement unit tests only | `SHARED-PLATFORM-001`; later IDE work is `CCM-IDE-001` |
| Unverified | Linux/macOS x86_64/ARM64 parity for TUI, paste, completion, editors, sandbox representation and packaging | cross-cutting | only Linux x86_64 artifact/test evidence exists | `SHARED-PLATFORM-001` |
| Planned | Provider-neutral projections and read-only orchestrator bridge | not implemented | no release claim | `SHARED-PROTO-001`, `SHARED-PROTO-002`, `AOR-BRIDGE-001`, `CCM-BRIDGE-001` |
| Planned | Claude provider, compare mode, durable bus and cross-provider skills | not implemented | no release claim | `AOR-ADAPTER-CLAUDE-001`, `CCM-ADAPTER-CLAUDE-001`, `AOR-COMPARE-001`, `AOR-BUS-001`, `SHARED-BUS-001` |
| Planned | IDE operator extension, observability, shared RAG, chat/SaaS and remote access | not implemented | no release claim | `CCM-IDE-001`, `SHARED-OBS-001`, `AOR-RAG-001`, `AOR-CHAT-001`, `AOR-MCP-001`, `SHARED-REMOTE-001`, `SHARED-REMOTE-002` |

## Baseline decision

`CCM-BASE-003` inventories and verifies the dirty candidate, but does **not**
close roadmap gate `G0`. Before `R0` can be called reproducible, the accumulated
work must be split/committed, the post-package fixes included in a fresh build,
release evidence captured, and the Linux/macOS baseline gaps resolved or
explicitly classified by `SHARED-PLATFORM-001`.
