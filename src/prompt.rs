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
}

#[derive(Clone, Debug)]
enum PromptKind {
    Approval {
        title: String,
        details: String,
        decisions: Vec<String>,
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
        can_accept: bool,
    },
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
    pub(crate) fn from_request(message: &Value) -> Result<Self, String> {
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
            },
            "item/fileChange/requestApproval" => PromptKind::Approval {
                title: "File change approval".to_string(),
                details: detail_lines(
                    params,
                    &["reason", "grantRoot", "itemId"],
                    Some("Codex requests permission to apply file changes."),
                ),
                decisions: vec![
                    "accept".to_string(),
                    "acceptForSession".to_string(),
                    "decline".to_string(),
                    "cancel".to_string(),
                ],
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
                    can_accept: mode == "url",
                }
            }
            _ => return Err(format!("unsupported server request: {method}")),
        };
        Ok(Self {
            request_id,
            thread_id,
            turn_id,
            kind,
        })
    }

    pub(crate) fn body(&self) -> String {
        match &self.kind {
            PromptKind::Approval {
                title,
                details,
                decisions,
            } => format!("{title}\n\n{details}\n\n{}", approval_help(decisions)),
            PromptKind::Permissions { details, .. } => {
                format!(
                    "Permission request\n\n{details}\n\n[y] allow once  [a] allow for session  [n] deny  [x] deny and interrupt"
                )
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
            PromptKind::Elicitation {
                details,
                can_accept,
            } => {
                let accept = if *can_accept {
                    "[y] completed/accept  "
                } else {
                    ""
                };
                format!("MCP elicitation\n\n{details}\n\n{accept}[n] decline  [x] cancel")
            }
        }
    }

    pub(crate) fn composer_title(&self) -> &'static str {
        match &self.kind {
            PromptKind::UserInput { .. } => " Answer required ",
            _ => " Decision required ",
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
        let request_id = self.request_id.clone();
        let result = match &mut self.kind {
            PromptKind::Approval { decisions, .. } => {
                approval_decision(key, decisions).map(|decision| json!({"decision": decision}))
            }
            PromptKind::Permissions { requested, .. } => match key.code {
                KeyCode::Char('y') => Some(json!({"permissions": requested, "scope": "turn"})),
                KeyCode::Char('a') => Some(json!({"permissions": requested, "scope": "session"})),
                KeyCode::Char('n') => Some(json!({"permissions": {}, "scope": "turn"})),
                KeyCode::Char('x') => {
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
            PromptKind::Elicitation { can_accept, .. } => match key.code {
                KeyCode::Char('y') if *can_accept => {
                    Some(json!({"action": "accept", "content": null}))
                }
                KeyCode::Char('n') => Some(json!({"action": "decline", "content": null})),
                KeyCode::Char('x') => Some(json!({"action": "cancel", "content": null})),
                _ => None,
            },
        };
        result.map(|result| PromptResolution {
            request_id,
            result,
            interrupt: None,
        })
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

fn approval_decision(key: KeyEvent, decisions: &[String]) -> Option<&'static str> {
    let decision = match key.code {
        KeyCode::Char('y') => "accept",
        KeyCode::Char('a') => "acceptForSession",
        KeyCode::Char('n') => "decline",
        KeyCode::Char('x') => "cancel",
        _ => return None,
    };
    decisions
        .iter()
        .any(|allowed| allowed == decision)
        .then_some(decision)
}

fn approval_help(decisions: &[String]) -> String {
    [
        ("accept", "[y] approve once"),
        ("acceptForSession", "[a] approve for session"),
        ("decline", "[n] decline"),
        ("cancel", "[x] decline and interrupt"),
    ]
    .into_iter()
    .filter(|(decision, _)| decisions.iter().any(|allowed| allowed == decision))
    .map(|(_, help)| help)
    .collect::<Vec<_>>()
    .join("  ")
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
