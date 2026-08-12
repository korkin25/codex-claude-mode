use std::ffi::OsString;
use std::fs;

use pretty_assertions::assert_eq;

use super::complete_with_path;

#[test]
fn completes_files_and_escapes_spaces_and_backslashes() {
    let root = test_directory("files");
    fs::write(root.join("two words"), "").expect("write file");
    fs::write(root.join("back\\slash"), "").expect("write file");
    fs::create_dir(root.join("topic")).expect("create directory");

    assert_eq!(
        complete_with_path("read t", &root, None).candidates,
        vec!["topic/".to_string(), "two\\ words".to_string()]
    );
    assert_eq!(
        complete_with_path("read back", &root, None).candidates,
        vec!["back\\\\slash".to_string()]
    );

    fs::remove_dir_all(root).expect("remove test directory");
}

#[cfg(unix)]
#[test]
fn first_token_uses_path_and_filters_non_executable_files() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_directory("path");
    let executable = root.join("hello world");
    fs::write(&executable, "").expect("write executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).expect("make executable");
    fs::write(root.join("hello-data"), "").expect("write non-executable");

    let path = OsString::from(root.as_os_str());
    assert_eq!(
        complete_with_path("hel", &root, Some(&path)).candidates,
        vec!["hello\\ world".to_string()]
    );

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn reports_byte_start_and_understands_escaped_whitespace() {
    let root = test_directory("token");
    fs::write(root.join("two words"), "").expect("write file");
    let completion = complete_with_path("run two\\ w", &root, None);
    assert_eq!(completion.start, 4);
    assert_eq!(completion.candidates, vec!["two\\ words".to_string()]);
    fs::remove_dir_all(root).expect("remove test directory");
}

fn test_directory(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "codex-claude-mode-completion-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create test directory");
    path
}
