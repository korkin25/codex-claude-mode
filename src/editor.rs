use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExternalEditor {
    Terminal,
    VsCode,
    Cursor,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EditorCommand {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
}

pub(crate) fn resolve_project_file(root: &Path, requested: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let candidate = fs::canonicalize(&candidate)
        .with_context(|| format!("failed to resolve file {}", candidate.display()))?;
    if !candidate.starts_with(&root) {
        bail!(
            "refusing to open a file outside the project: {}",
            candidate.display()
        );
    }
    if !candidate.is_file() {
        bail!("not a file: {}", candidate.display());
    }
    Ok(candidate)
}

pub(crate) fn command_for(
    editor: ExternalEditor,
    path: &Path,
    line: Option<u32>,
    column: Option<u32>,
) -> Result<EditorCommand> {
    match editor {
        ExternalEditor::Terminal => terminal_command(path, line),
        ExternalEditor::VsCode => Ok(goto_command("code", path, line, column)),
        ExternalEditor::Cursor => Ok(goto_command("cursor", path, line, column)),
    }
}

pub(crate) fn spawn_gui(command: &EditorCommand) -> Result<Child> {
    Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start {}", command.program.to_string_lossy()))
}

pub(crate) fn run_terminal(command: &EditorCommand) -> Result<()> {
    let status = Command::new(&command.program)
        .args(&command.args)
        .status()
        .with_context(|| format!("failed to start {}", command.program.to_string_lossy()))?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

fn terminal_command(path: &Path, line: Option<u32>) -> Result<EditorCommand> {
    let configured = env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vim".to_string());
    let mut words = split_command(&configured)?;
    let program = words.remove(0).into();
    if let Some(line) = line {
        words.push(format!("+{line}"));
    }
    words.push(path.as_os_str().to_string_lossy().into_owned());
    Ok(EditorCommand {
        program,
        args: words.into_iter().map(Into::into).collect(),
    })
}

fn goto_command(
    program: &str,
    path: &Path,
    line: Option<u32>,
    column: Option<u32>,
) -> EditorCommand {
    let target = match line {
        Some(line) => format!("{}:{line}:{}", path.display(), column.unwrap_or(1)),
        None => path.display().to_string(),
    };
    EditorCommand {
        program: program.into(),
        args: vec!["--goto".into(), target.into()],
    }
}

/// Parses fixed editor arguments without invoking a shell. Expansions, pipes,
/// redirects and command substitutions remain ordinary argument text.
fn split_command(value: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                word.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(character);
        }
    }
    if escaped || quote.is_some() {
        bail!("invalid VISUAL/EDITOR value: unmatched quote or escape");
    }
    if !word.is_empty() {
        words.push(word);
    }
    if words.is_empty() {
        bail!("VISUAL/EDITOR does not contain an executable");
    }
    Ok(words)
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
