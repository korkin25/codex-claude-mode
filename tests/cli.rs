#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("codex-claude-mode-cli-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove test directory");
    }
}

fn fake_codex(directory: &TempDir) -> (PathBuf, PathBuf) {
    let executable = directory.0.join("fake-codex");
    let arguments = directory.0.join("arguments");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CCM_ARGS_FILE\"\nprintf 'fake Codex help\\n'\nexit \"${CCM_EXIT_CODE:-0}\"\n",
    )
    .expect("write fake Codex");
    let mut permissions = fs::metadata(&executable)
        .expect("read fake Codex metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).expect("make fake Codex executable");
    (executable, arguments)
}

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_codex-claude-mode"))
}

#[test]
fn combined_help_forwards_codex_arguments_and_appends_help() {
    let directory = TempDir::new();
    let (codex, arguments) = fake_codex(&directory);
    let output = binary()
        .args([
            "--profile",
            "work",
            "--model=gpt-test",
            "--codex",
            codex.to_str().expect("UTF-8 test path"),
            "--help",
        ])
        .env("CCM_ARGS_FILE", &arguments)
        .output()
        .expect("run combined help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    assert!(stdout.contains("Show wrapper options"));
    assert!(stdout.contains("fake Codex help"));
    assert_eq!(
        fs::read_to_string(arguments).expect("read captured arguments"),
        "--profile\nwork\n--model=gpt-test\n--help\n"
    );
}

#[test]
fn combined_help_reports_codex_nonzero_exit() {
    let directory = TempDir::new();
    let (codex, arguments) = fake_codex(&directory);
    let output = binary()
        .args(["--codex", codex.to_str().expect("UTF-8 test path"), "-h"])
        .env("CCM_ARGS_FILE", &arguments)
        .env("CCM_EXIT_CODE", "23")
        .output()
        .expect("run combined help");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fake Codex help"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exit status: 23"));
    assert_eq!(
        fs::read_to_string(arguments).expect("read captured arguments"),
        "--help\n"
    );
}

#[test]
fn combined_help_forwards_non_utf8_codex_argument_unchanged() {
    let directory = TempDir::new();
    let (codex, arguments) = fake_codex(&directory);
    let non_utf8 = OsString::from_vec(vec![b'x', 0xff]);
    let output = binary()
        .args([
            OsString::from("--codex"),
            codex.into_os_string(),
            non_utf8,
            OsString::from("--help"),
        ])
        .env("CCM_ARGS_FILE", &arguments)
        .output()
        .expect("run combined help");

    assert!(output.status.success());
    assert_eq!(
        fs::read(arguments).expect("read captured arguments"),
        vec![b'x', 0xff, b'\n', b'-', b'-', b'h', b'e', b'l', b'p', b'\n']
    );
}

#[test]
fn combined_help_reports_missing_codex_executable() {
    let directory = TempDir::new();
    let missing = directory.0.join("missing-codex");
    let output = binary()
        .args([
            "--codex",
            missing.to_str().expect("UTF-8 test path"),
            "--help",
        ])
        .output()
        .expect("run combined help");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to run"));
}

#[test]
fn wrapper_value_options_report_missing_values() {
    for option in ["--codex", "--codex-home", "--cwd", "--thread"] {
        let output = binary().arg(option).output().expect("run wrapper");
        assert_eq!(output.status.code(), Some(2), "{option}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("a value is required"),
            "{option}"
        );
    }
}
