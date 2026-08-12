use std::path::Path;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;

pub(crate) struct CodexVersion {
    pub(crate) current: Option<String>,
    pub(crate) latest: Option<String>,
}

pub(crate) fn read(codex: &Path, codex_home: &Path) -> CodexVersion {
    let current = Command::new(codex)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| version_from_text(&output));
    let latest = std::fs::read_to_string(codex_home.join("version.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|value| value.get("latest_version")?.as_str().map(str::to_string));
    CodexVersion { current, latest }
}

pub(crate) fn update_available(current: &str, latest: &str) -> bool {
    let mut current = version_parts(current);
    let mut latest = version_parts(latest);
    let component_count = current.len().max(latest.len());
    current.resize(component_count, 0);
    latest.resize(component_count, 0);
    current < latest
}

pub(crate) fn run_update(codex: &Path, codex_home: &Path) -> Result<String> {
    let output = Command::new(codex)
        .arg("update")
        .env("CODEX_HOME", codex_home)
        .output()
        .with_context(|| format!("failed to run {} update", codex.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() {
            "Codex update completed".to_string()
        } else {
            stdout
        })
    } else {
        anyhow::bail!(if stderr.is_empty() {
            format!("Codex update exited with {}", output.status)
        } else {
            stderr
        })
    }
}

fn version_from_text(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|part| {
            part.trim_start_matches('v')
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        })
        .map(|part| part.trim_start_matches('v').to_string())
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .take_while(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
