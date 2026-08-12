use std::ffi::OsString;
use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use super::ExternalEditor;
use super::command_for;
use super::resolve_project_file;
use super::split_command;

#[test]
fn parses_editor_with_fixed_quoted_arguments_without_a_shell() {
    assert_eq!(
        split_command("nvim --cmd 'set mouse=' -f").unwrap(),
        vec!["nvim", "--cmd", "set mouse=", "-f"]
    );
    assert_eq!(
        split_command("vim '; touch /tmp/nope'").unwrap(),
        vec!["vim", "; touch /tmp/nope"]
    );
}

#[test]
fn rejects_malformed_editor_configuration() {
    assert!(split_command("vim '").is_err());
    assert!(split_command("vim \\").is_err());
}

#[test]
fn builds_gui_goto_as_distinct_arguments() {
    let command = command_for(
        ExternalEditor::Cursor,
        Path::new("/tmp/a file.rs"),
        Some(12),
        Some(4),
    )
    .unwrap();
    assert_eq!(command.program, OsString::from("cursor"));
    assert_eq!(
        command.args,
        vec![
            OsString::from("--goto"),
            OsString::from("/tmp/a file.rs:12:4")
        ]
    );
}

#[test]
fn project_file_resolution_rejects_symlink_escape_and_directories() {
    let root = std::env::temp_dir().join(format!("ccm-editor-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
    assert_eq!(
        resolve_project_file(&root, Path::new("src/main.rs")).unwrap(),
        fs::canonicalize(root.join("src/main.rs")).unwrap()
    );
    assert!(resolve_project_file(&root, Path::new("src")).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", root.join("escape")).unwrap();
        assert!(resolve_project_file(&root, Path::new("escape")).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}
