use std::ffi::OsString;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::json;

use super::codex_home_from_home;
use super::permission_update_params;
use super::split_args;
use super::turn_start_params;

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
            "continue the task",
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
