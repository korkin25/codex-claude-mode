use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use serde_json::Value;

pub(crate) const ROUTED_MESSAGE_PREFIX: &str = "Пользователь выбрал субагента ";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TokenUsage {
    pub(crate) input: i64,
    pub(crate) output: i64,
    pub(crate) total: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogKind {
    User,
    Agent,
    Activity,
}

#[derive(Clone, Debug)]
pub(crate) struct LogEntry {
    pub(crate) text: String,
    pub(crate) kind: LogKind,
    waiting_since: Option<Instant>,
    answered_after: Option<Duration>,
}

impl LogEntry {
    fn historical(text: String, kind: LogKind) -> Self {
        Self {
            text,
            kind,
            waiting_since: None,
            answered_after: None,
        }
    }

    fn pending_user(text: String) -> Self {
        Self {
            text,
            kind: LogKind::User,
            waiting_since: Some(Instant::now()),
            answered_after: None,
        }
    }

    pub(crate) fn timing_label(&self) -> Option<String> {
        let started = self.waiting_since?;
        let elapsed = self.answered_after.unwrap_or_else(|| started.elapsed());
        let label = if self.answered_after.is_some() {
            "answered in"
        } else {
            "waiting"
        };
        Some(format!("{label} {}", format_duration(elapsed)))
    }

    fn mark_answered(&mut self) {
        if self.answered_after.is_none()
            && let Some(started) = self.waiting_since
        {
            self.answered_after = Some(started.elapsed());
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AgentThread {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) cwd: String,
    pub(crate) label: String,
    pub(crate) preview: String,
    pub(crate) status: String,
    pub(crate) can_accept_direct_input: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) log: Vec<LogEntry>,
    pub(crate) tokens: TokenUsage,
    pub(crate) active_since: Option<Instant>,
    pub(crate) active_turn_id: Option<String>,
    activity: Option<String>,
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
        let created_at = value
            .get("createdAt")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let is_subagent = parent_id.is_some();
        Some(Self {
            id,
            session_id: string(value, "sessionId").unwrap_or_default(),
            parent_id,
            path: string(value, "path"),
            cwd: string(value, "cwd").unwrap_or_default(),
            label,
            preview: string(value, "preview").unwrap_or_default(),
            status,
            can_accept_direct_input: value
                .get("canAcceptDirectInput")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            created_at,
            updated_at: value
                .get("updatedAt")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            log: transcript(value, is_subagent, created_at),
            tokens: TokenUsage::default(),
            active_since,
            active_turn_id: active_turn_id(value),
            activity: None,
            live_items: HashMap::new(),
        })
    }

    pub(crate) fn merge_metadata(&mut self, value: &Value) {
        let replacement = Self::from_json(value);
        if let Some(replacement) = replacement {
            self.parent_id = replacement.parent_id;
            self.session_id = replacement.session_id;
            self.path = replacement.path;
            self.cwd = replacement.cwd;
            self.label = replacement.label;
            self.preview = replacement.preview;
            self.can_accept_direct_input = replacement.can_accept_direct_input;
            if replacement.created_at != 0 {
                self.created_at = replacement.created_at;
            }
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
            self.activity = None;
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

    pub(crate) fn display_status(&self) -> String {
        self.activity.as_ref().map_or_else(
            || self.status.clone(),
            |activity| format!("{} · {activity}", self.status),
        )
    }

    pub(crate) fn update_activity(&mut self, item: &Value) {
        self.activity = activity_name(item);
    }

    pub(crate) fn push_user_message(&mut self, text: String) {
        self.log.push(LogEntry::pending_user(text));
    }

    pub(crate) fn push_activity_message(&mut self, text: String) {
        self.log.push(LogEntry::historical(text, LogKind::Activity));
    }

    pub(crate) fn append_delta(&mut self, item_id: &str, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.mark_latest_user_answered();
        self.activity = Some("responding".to_string());
        let index = *self
            .live_items
            .entry(item_id.to_string())
            .or_insert_with(|| {
                self.log
                    .push(LogEntry::historical(String::new(), LogKind::Agent));
                self.log.len() - 1
            });
        self.log[index].text.push_str(delta);
    }

    pub(crate) fn update_item(&mut self, item: &Value, completed: bool) {
        let Some(rendered) = render_item(item) else {
            return;
        };
        if rendered.kind == LogKind::Agent {
            self.mark_latest_user_answered();
        }
        let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if let Some(index) = self.live_items.remove(item_id) {
            self.log[index] = rendered;
        } else if self
            .log
            .last()
            .is_none_or(|last| last.kind != rendered.kind || last.text != rendered.text)
        {
            self.log.push(rendered);
            if !completed && !item_id.is_empty() {
                self.live_items
                    .insert(item_id.to_string(), self.log.len() - 1);
            }
        }
    }

    fn mark_latest_user_answered(&mut self) {
        if let Some(entry) = self
            .log
            .iter_mut()
            .rev()
            .find(|entry| entry.kind == LogKind::User && entry.answered_after.is_none())
        {
            entry.mark_answered();
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

fn transcript(thread: &Value, is_subagent: bool, created_at: i64) -> Vec<LogEntry> {
    thread
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|turn| {
            !(is_routed_transport_turn(turn)
                || is_subagent
                    && turn
                        .get("startedAt")
                        .and_then(Value::as_i64)
                        .is_some_and(|started_at| started_at < created_at))
        })
        .flat_map(|turn| {
            turn.get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(render_item)
        })
        .collect()
}

pub(crate) fn render_item(item: &Value) -> Option<LogEntry> {
    match item.get("type")?.as_str()? {
        "userMessage" => {
            let text = user_message_text(item)?;
            Some(LogEntry::historical(text, LogKind::User))
        }
        "agentMessage" => Some(LogEntry::historical(
            nonempty_string(item, "text")?,
            LogKind::Agent,
        )),
        "reasoning" => None,
        "commandExecution" => activity_entry(command_text(item)?),
        "fileChange" => activity_entry(file_change_text(item)),
        "mcpToolCall" => activity_entry(tool_text("MCP", item)),
        "dynamicToolCall" => activity_entry(tool_text("Tool", item)),
        "collabAgentToolCall" => activity_entry(collab_text(item)),
        "subAgentActivity" => activity_entry(format!(
            "Sub-agent {}: {}",
            string(item, "kind").unwrap_or_else(|| "active".to_string()),
            string(item, "agentPath").unwrap_or_default()
        )),
        "webSearch" => activity_entry(web_search_text(item)),
        "imageView" => activity_entry(format!(
            "Viewed image: {}",
            string(item, "path").unwrap_or_default()
        )),
        "sleep" => activity_entry(format!(
            "Waiting: {} ms",
            item.get("durationMs")
                .and_then(Value::as_i64)
                .unwrap_or_default()
        )),
        "imageGeneration" => activity_entry("Generating image".to_string()),
        "plan" => activity_entry(format!("Plan: {}", string(item, "text")?)),
        "contextCompaction" => activity_entry("Compact context".to_string()),
        _ => None,
    }
}

fn activity_entry(text: String) -> Option<LogEntry> {
    (!text.trim().is_empty()).then(|| LogEntry::historical(text, LogKind::Activity))
}

fn command_text(item: &Value) -> Option<String> {
    let command = string(item, "command")?;
    let status = string(item, "status").unwrap_or_else(|| "inProgress".to_string());
    let action = item
        .get("commandActions")
        .and_then(Value::as_array)
        .and_then(|actions| actions.first());
    let heading = match action
        .and_then(|action| action.get("type"))
        .and_then(Value::as_str)
    {
        Some("read") => format!(
            "Read [{}]: {}",
            status,
            action
                .and_then(|action| string(action, "path"))
                .unwrap_or(command)
        ),
        Some("listFiles") => format!(
            "List files [{}]: {}",
            status,
            action
                .and_then(|action| string(action, "path"))
                .unwrap_or(command)
        ),
        Some("search") => format!(
            "Search files [{}]: {}",
            status,
            action
                .and_then(|action| string(action, "query"))
                .unwrap_or(command)
        ),
        Some("unknown") | None | Some(_) => format!("Command [{status}]: {command}"),
    };
    Some(heading)
}

fn file_change_text(item: &Value) -> String {
    let changes = item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| {
            let path = string(change, "path")?;
            let kind = string(change, "kind").unwrap_or_else(|| "update".to_string());
            Some(format!("{kind}: {path}"))
        })
        .collect::<Vec<_>>();
    if changes.is_empty() {
        "File changes".to_string()
    } else {
        format!("File changes: {}", changes.join(", "))
    }
}

fn tool_text(prefix: &str, item: &Value) -> String {
    let namespace = string(item, "server")
        .or_else(|| string(item, "namespace"))
        .filter(|namespace| !namespace.is_empty());
    let tool = string(item, "tool").unwrap_or_else(|| "unknown".to_string());
    let name = namespace.map_or(tool.clone(), |namespace| format!("{namespace}/{tool}"));
    let status = string(item, "status").unwrap_or_else(|| "inProgress".to_string());
    format!("{prefix} [{status}]: {name}")
}

fn collab_text(item: &Value) -> String {
    let tool = item.get("tool").and_then(Value::as_str).map_or_else(
        || item.get("tool").map_or_else(String::new, Value::to_string),
        str::to_string,
    );
    let status = string(item, "status").unwrap_or_else(|| "inProgress".to_string());
    let receivers = string_array(item, "receiverThreadIds").join(", ");
    if receivers.is_empty() {
        format!("Agent action [{status}]: {tool}")
    } else {
        format!("Agent action [{status}]: {tool} → {receivers}")
    }
}

fn web_search_text(item: &Value) -> String {
    let query = string(item, "query").or_else(|| {
        item.pointer("/action/query")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let action_type = item
        .pointer("/action/type")
        .and_then(Value::as_str)
        .unwrap_or("search");
    match action_type {
        "openPage" => format!(
            "Open web page: {}",
            item.pointer("/action/url")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ),
        "findInPage" => format!(
            "Find on web page: {}",
            item.pointer("/action/pattern")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ),
        "search" | "other" => format!("Web search: {}", query.unwrap_or_default()),
        _ => format!("Web action: {action_type}"),
    }
}

fn activity_name(item: &Value) -> Option<String> {
    match item.get("type")?.as_str()? {
        "reasoning" => Some("thinking".to_string()),
        "commandExecution" => Some("running command".to_string()),
        "fileChange" => Some("changing files".to_string()),
        "mcpToolCall" | "dynamicToolCall" => Some("using tool".to_string()),
        "webSearch" => Some("searching web".to_string()),
        "imageView" => Some("viewing image".to_string()),
        "collabAgentToolCall" => {
            let tool = item.get("tool")?;
            let name = tool
                .as_str()
                .map_or_else(|| tool.to_string(), str::to_string);
            Some(format!("agent action: {name}"))
        }
        "subAgentActivity" => Some(format!(
            "sub-agent: {}",
            string(item, "kind").unwrap_or_else(|| "active".to_string())
        )),
        "agentMessage" => Some("responding".to_string()),
        _ => None,
    }
}

fn is_routed_transport_turn(turn: &Value) -> bool {
    turn.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("userMessage"))
        .filter_map(user_message_text)
        .any(|text| text.starts_with(ROUTED_MESSAGE_PREFIX))
}

fn user_message_text(item: &Value) -> Option<String> {
    Some(
        item.get("content")?
            .as_array()?
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(ToOwned::to_owned)
}

fn nonempty_string(value: &Value, key: &str) -> Option<String> {
    string(value, key).filter(|text| !text.trim().is_empty())
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
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
