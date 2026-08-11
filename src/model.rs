use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TokenUsage {
    pub(crate) input: i64,
    pub(crate) output: i64,
    pub(crate) total: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentThread {
    pub(crate) id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) label: String,
    pub(crate) preview: String,
    pub(crate) status: String,
    pub(crate) can_accept_direct_input: bool,
    pub(crate) updated_at: i64,
    pub(crate) log: Vec<String>,
    pub(crate) tokens: TokenUsage,
    pub(crate) active_since: Option<Instant>,
    pub(crate) active_turn_id: Option<String>,
    live_items: HashMap<String, usize>,
}

impl AgentThread {
    pub(crate) fn from_json(value: &Value) -> Option<Self> {
        let id = value.get("id")?.as_str()?.to_owned();
        let parent_id = string(value, "parentThreadId");
        let nickname = string(value, "agentNickname");
        let role = string(value, "agentRole");
        let label = if parent_id.is_none() {
            "Main".to_string()
        } else {
            match (nickname, role) {
                (Some(name), Some(role)) => format!("{name} ({role})"),
                (Some(name), None) => name,
                (None, Some(role)) => role,
                (None, None) => short_id(&id),
            }
        };
        let status = status_name(value.get("status"));
        let active_since = (status == "working").then(Instant::now);
        Some(Self {
            id,
            parent_id,
            label,
            preview: string(value, "preview").unwrap_or_default(),
            status,
            can_accept_direct_input: value
                .get("canAcceptDirectInput")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            updated_at: value
                .get("updatedAt")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            log: transcript(value),
            tokens: TokenUsage::default(),
            active_since,
            active_turn_id: active_turn_id(value),
            live_items: HashMap::new(),
        })
    }

    pub(crate) fn merge_metadata(&mut self, value: &Value) {
        let replacement = Self::from_json(value);
        if let Some(replacement) = replacement {
            self.parent_id = replacement.parent_id;
            self.label = replacement.label;
            self.preview = replacement.preview;
            self.can_accept_direct_input = replacement.can_accept_direct_input;
            self.updated_at = replacement.updated_at;
            self.set_status(replacement.status);
            if replacement.active_turn_id.is_some() {
                self.active_turn_id = replacement.active_turn_id;
            }
            if !replacement.log.is_empty() {
                self.log = replacement.log;
                self.live_items.clear();
            }
        }
    }

    pub(crate) fn set_status(&mut self, status: String) {
        if status == "working" && self.active_since.is_none() {
            self.active_since = Some(Instant::now());
        } else if status != "working" {
            self.active_since = None;
        }
        self.status = status;
    }

    pub(crate) fn start_turn(&mut self, turn_id: String) {
        self.active_turn_id = Some(turn_id);
        self.set_status("working".to_string());
    }

    pub(crate) fn complete_turn(&mut self) {
        self.active_turn_id = None;
        self.set_status("idle".to_string());
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.active_since
            .map_or(Duration::ZERO, |start| start.elapsed())
    }

    pub(crate) fn append_delta(&mut self, item_id: &str, delta: &str) {
        let index = *self
            .live_items
            .entry(item_id.to_string())
            .or_insert_with(|| {
                self.log.push("Assistant: ".to_string());
                self.log.len() - 1
            });
        self.log[index].push_str(delta);
    }

    pub(crate) fn complete_item(&mut self, item: &Value) {
        let Some(rendered) = render_item(item) else {
            return;
        };
        let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if let Some(index) = self.live_items.remove(item_id) {
            self.log[index] = rendered;
        } else if self.log.last() != Some(&rendered) {
            self.log.push(rendered);
        }
    }
}

pub(crate) fn status_name(value: Option<&Value>) -> String {
    match value
        .and_then(|status| status.get("type"))
        .and_then(Value::as_str)
    {
        Some("active") => "working",
        Some("idle") => "idle",
        Some("notLoaded") => "closed",
        Some("systemError") => "error",
        Some(other) => other,
        None => "unknown",
    }
    .to_string()
}

fn transcript(thread: &Value) -> Vec<String> {
    thread
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|turn| {
            turn.get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(render_item)
        })
        .collect()
}

pub(crate) fn render_item(item: &Value) -> Option<String> {
    match item.get("type")?.as_str()? {
        "userMessage" => {
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            Some(format!("You: {text}"))
        }
        "agentMessage" => Some(format!("Assistant: {}", string(item, "text")?)),
        "reasoning" => Some(format!("Thinking: {}", strings(item, "summary").join("\n"))),
        "commandExecution" => {
            let command = string(item, "command")?;
            let status = string(item, "status").unwrap_or_default();
            let output = string(item, "aggregatedOutput").unwrap_or_default();
            Some(format!("Command [{status}]: {command}\n{output}"))
        }
        "fileChange" => Some("File changes applied".to_string()),
        "collabAgentToolCall" => Some(format!(
            "Agent action: {}",
            item.get("tool").map_or_else(String::new, Value::to_string)
        )),
        "subAgentActivity" => Some(format!(
            "Sub-agent activity: {}",
            string(item, "kind").unwrap_or_default()
        )),
        "plan" => Some(format!("Plan: {}", string(item, "text")?)),
        other => Some(format!("{other}: completed")),
    }
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(ToOwned::to_owned)
}

fn strings<'a>(value: &'a Value, key: &str) -> Vec<&'a str> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn active_turn_id(thread: &Value) -> Option<String> {
    thread
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))
        .and_then(|turn| string(turn, "id"))
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
