//! Side-effect-free, shell-style completion for the composer.
//!
//! This module never invokes a shell or sources startup files. It only reads
//! `PATH` and the filesystem relative to the caller-provided working directory.

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Completion {
    /// Byte offset at which the current token should be replaced.
    pub(crate) start: usize,
    /// Sorted, deduplicated, shell-escaped replacement strings.
    pub(crate) candidates: Vec<String>,
}

pub(crate) fn complete(input: &str, cwd: &Path) -> Completion {
    complete_with_path(input, cwd, env::var_os("PATH").as_deref())
}

fn complete_with_path(input: &str, cwd: &Path, path: Option<&OsStr>) -> Completion {
    let start = current_token_start(input);
    let prefix = unescape_token(&input[start..]);
    let first_token = input[..start].trim().is_empty();
    let candidates = if first_token && !prefix.contains('/') {
        command_candidates(&prefix, path)
    } else {
        filesystem_candidates(&prefix, cwd)
    };
    Completion { start, candidates }
}

fn current_token_start(input: &str) -> usize {
    let mut start = 0;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character.is_whitespace() {
            start = index + character.len_utf8();
        }
    }
    start
}

fn unescape_token(token: &str) -> String {
    let mut result = String::with_capacity(token.len());
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                result.push(next);
            } else {
                result.push(character);
            }
        } else {
            result.push(character);
        }
    }
    result
}

fn command_candidates(prefix: &str, path: Option<&OsStr>) -> Vec<String> {
    let Some(path) = path else {
        return Vec::new();
    };
    let mut candidates = BTreeSet::new();
    for directory in env::split_paths(path) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with(prefix) && is_executable(&entry.path()) {
                candidates.insert(escape_token(name));
            }
        }
    }
    candidates.into_iter().collect()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn filesystem_candidates(prefix: &str, cwd: &Path) -> Vec<String> {
    let (parent_text, name_prefix) = match prefix.rsplit_once('/') {
        Some((parent, name)) => (Some(parent), name),
        None => (None, prefix),
    };
    let lookup_parent = match parent_text {
        Some("") => PathBuf::from("/"),
        Some(parent) => cwd.join(parent),
        None => cwd.to_path_buf(),
    };
    let Ok(entries) = std::fs::read_dir(lookup_parent) else {
        return Vec::new();
    };
    let show_hidden = name_prefix.starts_with('.');
    let mut candidates = BTreeSet::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if (!show_hidden && name.starts_with('.')) || !name.starts_with(name_prefix) {
            continue;
        }
        let mut replacement = match parent_text {
            Some("") => format!("/{name}"),
            Some(parent) => format!("{parent}/{name}"),
            None => name.to_string(),
        };
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            replacement.push('/');
        }
        candidates.insert(escape_token(&replacement));
    }
    candidates.into_iter().collect()
}

fn escape_token(token: &str) -> String {
    let mut escaped = String::with_capacity(token.len());
    for character in token.chars() {
        if character.is_whitespace() || character == '\\' {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
#[path = "shell_completion_tests.rs"]
mod tests;
