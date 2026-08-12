use std::ffi::OsString;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::json;

use super::codex_home_from_home;
use super::enter_terminal;
use super::leave_terminal;
use super::permission_update_params;
use super::resume_terminal_features;
use super::split_args;
use super::suspend_terminal_features;
use super::turn_start_params;
use crate::ui::SubmissionInput;

#[test]
fn terminal_lifecycle_enables_and_disables_bracketed_paste() {
    let mut entered = Vec::new();
    enter_terminal(&mut entered).unwrap();
    assert!(entered.windows(8).any(|window| window == b"\x1b[?2004h"));

    let mut left = Vec::new();
    leave_terminal(&mut left).unwrap();
    assert!(left.windows(8).any(|window| window == b"\x1b[?2004l"));
}

#[test]
fn terminal_editor_suspend_and_resume_toggle_bracketed_paste() {
    let mut suspended = Vec::new();
    suspend_terminal_features(&mut suspended).unwrap();
    assert!(suspended.windows(8).any(|window| window == b"\x1b[?2004l"));

    let mut resumed = Vec::new();
    resume_terminal_features(&mut resumed).unwrap();
    assert!(resumed.windows(8).any(|window| window == b"\x1b[?2004h"));
}

#[test]
fn splits_wrapper_and_codex_options() {
    let (wrapper, codex) = split_args(
        [
            "ccm",
            "--codex-home",
            "/tmp/test-home",
            "--model",
            "gpt-test",
            "--",
            "--profile",
            "work",
        ]
        .map(OsString::from),
    );

    assert_eq!(
        wrapper,
        ["ccm", "--codex-home", "/tmp/test-home"].map(OsString::from)
    );
    assert_eq!(
        codex,
        ["--model", "gpt-test", "--profile", "work"].map(OsString::from)
    );
}

#[test]
fn keeps_both_help_spellings_in_wrapper_arguments() {
    for help in ["-h", "--help"] {
        let (wrapper, codex) = split_args(["ccm", help].map(OsString::from));
        assert_eq!(wrapper, ["ccm", help].map(OsString::from));
        assert_eq!(codex, Vec::<OsString>::new());
    }
}

#[test]
fn default_codex_home_uses_the_standard_home_directory() {
    assert_eq!(
        codex_home_from_home(PathBuf::from("/users/example")),
        PathBuf::from("/users/example/.codex")
    );
}

#[test]
fn permission_selection_updates_the_existing_backend_thread() {
    assert_eq!(
        permission_update_params("child-thread", ":danger-full-access"),
        json!({
            "threadId": "child-thread",
            "permissions": ":danger-full-access"
        })
    );
}

#[test]
fn next_turn_keeps_the_selected_permission_profile() {
    assert_eq!(
        turn_start_params(
            "resumed-child",
            &[SubmissionInput::Text("continue the task".to_string())],
            Some(":danger-full-access")
        ),
        json!({
            "threadId": "resumed-child",
            "input": [{
                "type": "text",
                "text": "continue the task",
                "textElements": []
            }],
            "permissions": ":danger-full-access"
        })
    );
}

#[test]
fn turn_input_forwards_skill_with_real_app_server_shape() {
    assert_eq!(
        turn_start_params(
            "thread",
            &[
                SubmissionInput::Text("Use $review".to_string()),
                SubmissionInput::Skill {
                    name: "review".to_string(),
                    path: PathBuf::from("/skills/review/SKILL.md"),
                },
            ],
            None,
        ),
        json!({
            "threadId": "thread",
            "input": [
                {"type": "text", "text": "Use $review", "textElements": []},
                {"type": "skill", "name": "review", "path": "/skills/review/SKILL.md"}
            ],
            "permissions": null
        })
    );
}

#[test]
fn turn_input_preserves_text_and_local_image_order() {
    assert_eq!(
        turn_start_params(
            "thread",
            &[
                SubmissionInput::Text("before".to_string()),
                SubmissionInput::LocalImage(PathBuf::from("/tmp/image.png")),
                SubmissionInput::Text("after".to_string()),
            ],
            None,
        ),
        json!({
            "threadId": "thread",
            "input": [
                {"type": "text", "text": "before", "textElements": []},
                {"type": "localImage", "path": "/tmp/image.png"},
                {"type": "text", "text": "after", "textElements": []}
            ],
            "permissions": null
        })
    );
}
