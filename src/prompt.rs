use std::collections::HashMap;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use serde_json::Value;
use serde_json::json;

#[derive(Clone, Debug)]
pub(crate) struct PromptResolution {
    pub(crate) request_id: Value,
    pub(crate) result: Value,
    pub(crate) interrupt: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServerPrompt {
    pub(crate) request_id: Value,
    pub(crate) thread_id: String,
    turn_id: Option<String>,
    kind: PromptKind,
    selected_decision: usize,
}

pub(crate) struct DecisionLine {
    pub(crate) text: String,
    pub(crate) selected: bool,
}

#[derive(Clone, Debug)]
enum PromptKind {
    Approval {
        title: String,
        details: String,
        decisions: Vec<String>,
        scope: ApprovalScope,
        patch: Option<String>,
    },
    Permissions {
        details: String,
        requested: Value,
    },
    UserInput {
        questions: Vec<Question>,
        current: usize,
        answers: HashMap<String, Value>,
    },
    Elicitation {
        details: String,
        accept_content: Option<Value>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalScope {
    Command,
    FileChange,
}

#[derive(Clone, Debug)]
struct Question {
    id: String,
    header: String,
    text: String,
    options: Vec<String>,
    secret: bool,
}

impl ServerPrompt {
    pub(crate) fn from_request_with_item(
        message: &Value,
        related_item: Option<&Value>,
    ) -> Result<Self, String> {
        let request_id = message
            .get("id")
            .cloned()
            .ok_or_else(|| "server request is missing id".to_string())?;
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "server request is missing method".to_string())?;
        let params = message
            .get("params")
            .ok_or_else(|| "server request is missing params".to_string())?;
        let thread_id = text(params, "threadId").unwrap_or_default();
        let turn_id = text(params, "turnId");
        let kind = match method {
            "item/commandExecution/requestApproval" => PromptKind::Approval {
                title: "Command approval".to_string(),
                details: command_details(params),
                decisions: decisions(params, &["accept", "acceptForSession", "decline", "cancel"]),
                scope: ApprovalScope::Command,
                patch: None,
            },
            "item/fileChange/requestApproval" => PromptKind::Approval {
                title: "File change approval".to_string(),
                details: file_change_details(params, related_item),
                decisions: vec![
                    "accept".to_string(),
                    "acceptForSession".to_string(),
                    "decline".to_string(),
                    "cancel".to_string(),
                ],
                scope: ApprovalScope::FileChange,
                patch: file_change_patch(related_item),
            },
            "item/permissions/requestApproval" => PromptKind::Permissions {
                details: format!(
                    "{}\nRequested permissions:\n{}",
                    text(params, "reason").unwrap_or_else(|| "Additional access requested".into()),
                    bounded_json(params.get("permissions").unwrap_or(&Value::Null))
                ),
                requested: params
                    .get("permissions")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            },
            "item/tool/requestUserInput" => PromptKind::UserInput {
                questions: parse_questions(params)?,
                current: 0,
                answers: HashMap::new(),
            },
            "mcpServer/elicitation/request" => {
                let mode = text(params, "mode").unwrap_or_else(|| "unknown".into());
                let details = detail_lines(
                    params,
                    &["serverName", "mode", "message", "url"],
                    Some("MCP server requests user interaction."),
                );
                PromptKind::Elicitation {
                    details,
                    accept_content: match mode.as_str() {
                        "url" => Some(Value::Null),
                        "form" if has_empty_form_schema(params) => Some(json!({})),
                        _ => None,
                    },
                }
            }
            _ => return Err(format!("unsupported server request: {method}")),
        };
        Ok(Self {
            request_id,
            thread_id,
            turn_id,
            kind,
            selected_decision: 0,
        })
    }

    pub(crate) fn body(&self) -> String {
        match &self.kind {
            PromptKind::Approval {
                title,
                details,
                scope,
                ..
            } => match scope {
                ApprovalScope::FileChange => {
                    format!("{title}\n\n{details}\n\nPress Ctrl-A to review the full patch.")
                }
                ApprovalScope::Command => format!("{title}\n\n{details}"),
            },
            PromptKind::Permissions { details, .. } => {
                format!("Permission request\n\n{details}")
            }
            PromptKind::UserInput {
                questions, current, ..
            } => {
                let question = &questions[*current];
                let options = if question.options.is_empty() {
                    String::new()
                } else {
                    format!("\nOptions: {}", question.options.join(" | "))
                };
                format!(
                    "Input requested ({}/{})\n\n{}\n{}{}\n\nType an answer and press Enter. Ctrl-C cancels the turn.",
                    current + 1,
                    questions.len(),
                    question.header,
                    question.text,
                    options
                )
            }
            PromptKind::Elicitation { details, .. } => {
                format!("MCP elicitation\n\n{details}")
            }
        }
    }

    pub(crate) fn decision_text(&self) -> Option<String> {
        self.decision_lines().map(|lines| {
            lines
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub(crate) fn decision_lines(&self) -> Option<Vec<DecisionLine>> {
        let (header, choices, footer) = match &self.kind {
            PromptKind::Approval {
                decisions, scope, ..
            } => (
                (*scope == ApprovalScope::FileChange)
                    .then_some("Would you like to make the following edits?"),
                approval_choices(decisions, *scope),
                Some(if *scope == ApprovalScope::FileChange {
                    "Ctrl-A review patch"
                } else {
                    "PgUp/PgDn scroll details"
                }),
            ),
            PromptKind::Permissions { .. } => (None, permission_choices(), None),
            PromptKind::Elicitation { accept_content, .. } => {
                (None, elicitation_choices(accept_content.is_some()), None)
            }
            PromptKind::UserInput { .. } => return None,
        };
        let mut lines = Vec::new();
        if let Some(header) = header {
            lines.push(DecisionLine {
                text: header.to_string(),
                selected: false,
            });
        }
        lines.extend(
            choices
                .into_iter()
                .enumerate()
                .map(|(index, (_, label))| DecisionLine {
                    text: format!(
                        "{} {label}",
                        if index == self.selected_decision {
                            "▶"
                        } else {
                            " "
                        }
                    ),
                    selected: index == self.selected_decision,
                }),
        );
        if let Some(footer) = footer {
            lines.push(DecisionLine {
                text: footer.to_string(),
                selected: false,
            });
        }
        Some(lines)
    }

    pub(crate) fn patch_text(&self) -> Option<&str> {
        match &self.kind {
            PromptKind::Approval { patch, .. } => patch.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn composer_title(&self) -> &'static str {
        match &self.kind {
            PromptKind::UserInput { .. } => " Answer required ",
            _ => " Approval required ",
        }
    }

    pub(crate) fn masks_input(&self) -> bool {
        match &self.kind {
            PromptKind::UserInput {
                questions, current, ..
            } => questions[*current].secret,
            _ => false,
        }
    }

    pub(crate) fn accepts_text(&self) -> bool {
        matches!(&self.kind, PromptKind::UserInput { .. })
    }

    pub(crate) fn handle_key(
        &mut self,
        key: KeyEvent,
        input: &mut String,
    ) -> Option<PromptResolution> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(self.resolve_and_interrupt(self.cancel_result()));
        }
        if !self.accepts_text() {
            let choice_count = self.decision_choices().len();
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selected_decision = self.selected_decision.saturating_sub(1);
                    return None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected_decision =
                        (self.selected_decision + 1).min(choice_count.saturating_sub(1));
                    return None;
                }
                _ => {}
            }
        }
        let request_id = self.request_id.clone();
        let result = match &mut self.kind {
            PromptKind::Approval {
                decisions, scope, ..
            } => approval_decision(key, decisions, *scope, self.selected_decision)
                .map(|decision| json!({"decision": decision})),
            PromptKind::Permissions { requested, .. } => match key.code {
                KeyCode::Char('y') => Some(json!({"permissions": requested, "scope": "turn"})),
                KeyCode::Char('a') => Some(json!({"permissions": requested, "scope": "session"})),
                KeyCode::Char('n') => Some(json!({"permissions": {}, "scope": "turn"})),
                KeyCode::Char('x') => {
                    return Some(
                        self.resolve_and_interrupt(json!({"permissions": {}, "scope": "turn"})),
                    );
                }
                KeyCode::Enter if self.selected_decision == 0 => {
                    Some(json!({"permissions": requested, "scope": "turn"}))
                }
                KeyCode::Enter if self.selected_decision == 1 => {
                    Some(json!({"permissions": requested, "scope": "session"}))
                }
                KeyCode::Enter if self.selected_decision == 2 => {
                    Some(json!({"permissions": {}, "scope": "turn"}))
                }
                KeyCode::Enter if self.selected_decision == 3 => {
                    return Some(
                        self.resolve_and_interrupt(json!({"permissions": {}, "scope": "turn"})),
                    );
                }
                _ => None,
            },
            PromptKind::UserInput {
                questions,
                current,
                answers,
            } => match key.code {
                KeyCode::Backspace => {
                    input.pop();
                    None
                }
                KeyCode::Char(character) => {
                    input.push(character);
                    None
                }
                KeyCode::Enter if !input.trim().is_empty() => {
                    let answer = std::mem::take(input);
                    answers.insert(questions[*current].id.clone(), json!({"answers": [answer]}));
                    *current += 1;
                    if *current == questions.len() {
                        Some(json!({"answers": answers}))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            PromptKind::Elicitation { accept_content, .. } => match key.code {
                KeyCode::Char('y') if accept_content.is_some() => Some(json!({
                    "action": "accept",
                    "content": accept_content.clone().expect("accept content checked above")
                })),
                KeyCode::Char('n') => Some(json!({"action": "decline", "content": null})),
                KeyCode::Char('x') => Some(json!({"action": "cancel", "content": null})),
                KeyCode::Enter if accept_content.is_some() && self.selected_decision == 0 => {
                    Some(json!({
                        "action": "accept",
                        "content": accept_content.clone().expect("accept content checked above")
                    }))
                }
                KeyCode::Enter
                    if self.selected_decision == usize::from(accept_content.is_some()) =>
                {
                    Some(json!({"action": "decline", "content": null}))
                }
                KeyCode::Enter
                    if self.selected_decision == usize::from(accept_content.is_some()) + 1 =>
                {
                    Some(json!({"action": "cancel", "content": null}))
                }
                _ => None,
            },
        };
        result.map(|result| PromptResolution {
            request_id,
            result,
            interrupt: None,
        })
    }

    fn decision_choices(&self) -> Vec<(&str, &str)> {
        match &self.kind {
            PromptKind::Approval {
                decisions, scope, ..
            } => approval_choices(decisions, *scope),
            PromptKind::Permissions { .. } => permission_choices(),
            PromptKind::Elicitation { accept_content, .. } => {
                elicitation_choices(accept_content.is_some())
            }
            PromptKind::UserInput { .. } => Vec::new(),
        }
    }

    fn cancel_result(&self) -> Value {
        match &self.kind {
            PromptKind::Approval { .. } => json!({"decision": "cancel"}),
            PromptKind::Permissions { .. } => json!({"permissions": {}, "scope": "turn"}),
            PromptKind::UserInput { .. } => json!({"answers": {}}),
            PromptKind::Elicitation { .. } => json!({"action": "cancel", "content": null}),
        }
    }

    fn resolve_and_interrupt(&self, result: Value) -> PromptResolution {
        PromptResolution {
            request_id: self.request_id.clone(),
            result,
            interrupt: self
                .turn_id
                .clone()
                .map(|turn_id| (self.thread_id.clone(), turn_id)),
        }
    }
}

fn approval_decision(
    key: KeyEvent,
    decisions: &[String],
    scope: ApprovalScope,
    selected: usize,
) -> Option<String> {
    if key.code == KeyCode::Enter {
        return approval_choices(decisions, scope)
            .get(selected)
            .map(|(decision, _)| (*decision).to_string());
    }
    let decision = match key.code {
        KeyCode::Char('y') => "accept",
        KeyCode::Char('a') => "acceptForSession",
        KeyCode::Char('n') => "decline",
        KeyCode::Char('x') => "cancel",
        KeyCode::Esc if scope == ApprovalScope::FileChange => "cancel",
        _ => return None,
    };
    decisions
        .iter()
        .any(|allowed| allowed == decision)
        .then(|| decision.to_string())
}

fn approval_choices(decisions: &[String], scope: ApprovalScope) -> Vec<(&str, &str)> {
    let accept_for_session = match scope {
        ApprovalScope::Command => "[a] approve command for session",
        ApprovalScope::FileChange => "[a] Yes, and don't ask again for these files",
    };
    [
        (
            "accept",
            if scope == ApprovalScope::FileChange {
                "[y] Yes, proceed"
            } else {
                "[y] approve once"
            },
        ),
        ("acceptForSession", accept_for_session),
        (
            "decline",
            if scope == ApprovalScope::FileChange {
                "[n] No"
            } else {
                "[n] decline"
            },
        ),
        (
            "cancel",
            if scope == ApprovalScope::FileChange {
                "[Esc/x] No, and stop"
            } else {
                "[x] decline and stop"
            },
        ),
    ]
    .into_iter()
    .filter(|(decision, _)| decisions.iter().any(|allowed| allowed == decision))
    .collect()
}

fn permission_choices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("turn", "[y] allow once"),
        ("session", "[a] allow for session"),
        ("deny", "[n] deny"),
        ("cancel", "[x] deny and stop"),
    ]
}

fn elicitation_choices(can_accept: bool) -> Vec<(&'static str, &'static str)> {
    let mut choices = Vec::new();
    if can_accept {
        choices.push(("accept", "[y] accept"));
    }
    choices.push(("decline", "[n] decline"));
    choices.push(("cancel", "[x] cancel"));
    choices
}

fn has_empty_form_schema(params: &Value) -> bool {
    let Some(schema) = params.get("requestedSchema").and_then(Value::as_object) else {
        return false;
    };
    let properties_are_empty = schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty);
    let required_is_empty = schema
        .get("required")
        .map(|required| required.as_array().is_some_and(Vec::is_empty))
        .unwrap_or(true);

    schema.get("type").and_then(Value::as_str) == Some("object")
        && properties_are_empty
        && required_is_empty
}

fn decisions(params: &Value, fallback: &[&str]) -> Vec<String> {
    match params.get("availableDecisions").and_then(Value::as_array) {
        Some(values) => {
            let decisions = values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if decisions.is_empty() {
                vec!["decline".to_string(), "cancel".to_string()]
            } else {
                decisions
            }
        }
        None => fallback.iter().map(|value| (*value).to_string()).collect(),
    }
}

fn parse_questions(params: &Value) -> Result<Vec<Question>, String> {
    let questions = params
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| "requestUserInput has no questions".to_string())?
        .iter()
        .filter_map(|value| {
            Some(Question {
                id: text(value, "id")?,
                header: text(value, "header").unwrap_or_else(|| "Question".into()),
                text: text(value, "question")?,
                options: value
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|option| text(option, "label"))
                    .collect(),
                secret: value
                    .get("isSecret")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    if questions.is_empty() {
        return Err("requestUserInput contains no valid questions".to_string());
    }
    Ok(questions)
}

fn command_details(params: &Value) -> String {
    let mut details = detail_lines(params, &["command", "cwd", "reason"], None);
    if let Some(permissions) = params.get("additionalPermissions") {
        details.push_str("\nAdditional permissions:\n");
        details.push_str(&bounded_json(permissions));
    }
    details
}

fn file_change_details(params: &Value, item: Option<&Value>) -> String {
    let mut sections = Vec::new();
    if let Some(reason) = text(params, "reason") {
        sections.push(format!("Reason: {reason}"));
    }
    if let Some(grant_root) = text(params, "grantRoot") {
        sections.push(format!("Requested session write root: {grant_root}"));
    }

    let paths = item
        .and_then(|item| item.get("changes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| text(change, "path"))
        .collect::<Vec<_>>();
    if !paths.is_empty() {
        sections.push(format!("Files: {}", paths.join(", ")));
    }
    if sections.is_empty() {
        sections.push(format!(
            "The backend did not provide paths or a diff for item {}. Decline unless the change is already clear from the activity log.",
            text(params, "itemId").unwrap_or_else(|| "(unknown)".to_string())
        ));
    }
    sections.join("\n\n")
}

fn file_change_patch(item: Option<&Value>) -> Option<String> {
    let rendered = item?
        .get("changes")?
        .as_array()?
        .iter()
        .filter_map(render_file_change)
        .collect::<Vec<_>>();
    (!rendered.is_empty()).then(|| rendered.join("\n\n"))
}

fn render_file_change(change: &Value) -> Option<String> {
    let path = text(change, "path")?;
    let kind = change
        .get("kind")
        .and_then(|kind| {
            kind.as_str()
                .map(str::to_string)
                .or_else(|| kind.get("type").and_then(Value::as_str).map(str::to_string))
        })
        .unwrap_or_else(|| "update".to_string());
    let mut rendered = format!("{kind}: {path}");
    if let Some(diff) = text(change, "diff").filter(|diff| !diff.trim().is_empty()) {
        rendered.push('\n');
        rendered.push_str(&diff);
    }
    Some(rendered)
}

fn detail_lines(params: &Value, keys: &[&str], fallback: Option<&str>) -> String {
    let lines = keys
        .iter()
        .filter_map(|key| text(params, key).map(|value| format!("{key}: {value}")))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        fallback.unwrap_or("No details supplied.").to_string()
    } else {
        lines.join("\n")
    }
}

fn bounded_json(value: &Value) -> String {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    rendered.chars().take(4_000).collect()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(ToOwned::to_owned)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
