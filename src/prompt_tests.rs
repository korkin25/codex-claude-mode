use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::ServerPrompt;

#[test]
fn command_approval_honors_available_decisions() {
    let message = json!({
        "id": 7,
        "method": "item/commandExecution/requestApproval",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "itemId": "item",
            "command": "cargo test",
            "availableDecisions": ["accept", "decline"]
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let mut input = String::new();

    assert!(
        prompt
            .handle_key(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                &mut input
            )
            .is_none()
    );
    let resolution = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            &mut input,
        )
        .expect("decline");
    assert_eq!(resolution.request_id, json!(7));
    assert_eq!(resolution.result, json!({"decision": "decline"}));
}

#[test]
fn approval_default_and_navigation_resolve_the_highlighted_available_choice() {
    let message = json!({
        "id": 8,
        "method": "item/commandExecution/requestApproval",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "command": "cargo test",
            "availableDecisions": ["accept", "acceptForSession", "decline"]
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let default = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("default decision");
    assert_eq!(default.result, json!({"decision": "accept"}));

    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    assert!(
        prompt
            .handle_key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut String::new()
            )
            .is_none()
    );
    let selected = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("highlighted decision");
    assert_eq!(selected.result, json!({"decision": "acceptForSession"}));
}

#[test]
fn approval_default_never_selects_an_unavailable_decision() {
    let message = json!({
        "id": 9,
        "method": "item/commandExecution/requestApproval",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "availableDecisions": ["decline", "cancel"]
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let lines = prompt.decision_lines().expect("decision lines");
    assert!(
        lines
            .iter()
            .any(|line| line.selected && line.text.contains("decline"))
    );
    assert!(!lines.iter().any(|line| line.text.contains("approve")));

    let resolution = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("safe available default");
    assert_eq!(resolution.result, json!({"decision": "decline"}));
}

#[test]
fn permission_request_defaults_to_allow_once_and_can_select_deny() {
    let message = json!({
        "id": 10,
        "method": "item/permissions/requestApproval",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "permissions": {"fileSystem": {"write": ["/workspace"]}}
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let default = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("allow-once default");
    assert_eq!(default.result["scope"], "turn");
    assert_eq!(
        default.result["permissions"],
        message["params"]["permissions"]
    );

    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    prompt.handle_key(
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        &mut String::new(),
    );
    prompt.handle_key(
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        &mut String::new(),
    );
    let denied = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("deny selection");
    assert_eq!(denied.result, json!({"permissions": {}, "scope": "turn"}));
}

#[test]
fn user_input_collects_all_questions() {
    let message = json!({
        "id": "request",
        "method": "item/tool/requestUserInput",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "itemId": "item",
            "isBlocking": true,
            "questions": [
                {"id": "first", "header": "One", "question": "First?"},
                {"id": "second", "header": "Two", "question": "Second?", "isSecret": true}
            ]
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let mut input = "alpha".to_string();
    assert!(
        prompt
            .handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut input
            )
            .is_none()
    );
    assert!(prompt.masks_input());
    input = "beta".to_string();
    let resolution = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut input,
        )
        .expect("all answers");
    assert_eq!(
        resolution.result,
        json!({
            "answers": {
                "first": {"answers": ["alpha"]},
                "second": {"answers": ["beta"]}
            }
        })
    );
}

#[test]
fn empty_form_elicitation_accepts_with_empty_content_by_key_or_default() {
    let message = json!({
        "id": "elicitation",
        "method": "mcpServer/elicitation/request",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "serverName": "telegram",
            "mode": "form",
            "message": "Allow the telegram MCP server to run tool telegram_call?",
            "requestedSchema": {"type": "object", "properties": {}}
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    assert!(
        prompt
            .decision_text()
            .expect("decision text")
            .contains("[y] accept")
    );
    let accepted_by_key = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("accept by key");
    assert_eq!(
        accepted_by_key.result,
        json!({"action": "accept", "content": {}})
    );

    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let accepted_by_default = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("accept by default");
    assert_eq!(
        accepted_by_default.result,
        json!({"action": "accept", "content": {}})
    );
}

#[test]
fn nonempty_form_elicitation_stays_fail_closed() {
    let message = json!({
        "id": "elicitation",
        "method": "mcpServer/elicitation/request",
        "params": {
            "threadId": "thread",
            "serverName": "example",
            "mode": "form",
            "message": "Provide a value",
            "requestedSchema": {
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"]
            }
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    assert!(
        !prompt
            .decision_text()
            .expect("decision text")
            .contains("[y] accept")
    );
    assert!(
        prompt
            .handle_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                &mut String::new()
            )
            .is_none()
    );
}

#[test]
fn openai_form_elicitation_stays_fail_closed() {
    let message = json!({
        "id": "elicitation",
        "method": "mcpServer/elicitation/request",
        "params": {
            "threadId": "thread",
            "serverName": "example",
            "mode": "openai/form",
            "message": "Provide a value",
            "requestedSchema": {"type": "object", "properties": {}}
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    assert!(
        !prompt
            .decision_text()
            .expect("decision text")
            .contains("[y] accept")
    );
    assert!(
        prompt
            .handle_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                &mut String::new()
            )
            .is_none()
    );
}

#[test]
fn url_elicitation_still_accepts_with_null_content() {
    let message = json!({
        "id": "elicitation",
        "method": "mcpServer/elicitation/request",
        "params": {
            "threadId": "thread",
            "serverName": "example",
            "mode": "url",
            "message": "Open the authorization URL",
            "url": "https://example.com/authorize"
        }
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let accepted = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            &mut String::new(),
        )
        .expect("accept URL elicitation");
    assert_eq!(
        accepted.result,
        json!({"action": "accept", "content": null})
    );
}

#[test]
fn control_c_cancels_prompt_and_interrupts_turn() {
    let message = json!({
        "id": 9,
        "method": "item/fileChange/requestApproval",
        "params": {"threadId": "thread", "turnId": "turn", "itemId": "item"}
    });
    let mut prompt = ServerPrompt::from_request_with_item(&message, None).expect("valid prompt");
    let resolution = prompt
        .handle_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &mut String::new(),
        )
        .expect("cancel");

    assert_eq!(resolution.result, json!({"decision": "cancel"}));
    assert_eq!(
        resolution.interrupt,
        Some(("thread".to_string(), "turn".to_string()))
    );
}

#[test]
fn file_change_approval_shows_paths_diff_reason_and_decision_scope() {
    let message = json!({
        "id": 12,
        "method": "item/fileChange/requestApproval",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "itemId": "patch",
            "reason": "Implement the requested theme",
            "grantRoot": "/workspace"
        }
    });
    let item = json!({
        "type": "fileChange",
        "id": "patch",
        "changes": [{
            "path": "src/ui.rs",
            "kind": {"type": "update", "movePath": null},
            "diff": "@@ -1 +1 @@\n-old\n+new"
        }]
    });

    let prompt = ServerPrompt::from_request_with_item(&message, Some(&item)).expect("valid prompt");
    let body = prompt.body();

    assert!(body.contains("Reason: Implement the requested theme"));
    assert!(body.contains("Requested session write root: /workspace"));
    assert!(body.contains("Files: src/ui.rs"));
    assert!(!body.contains("@@ -1 +1 @@\n-old\n+new"));
    let patch = prompt.patch_text().expect("full patch");
    assert!(patch.contains("update: src/ui.rs"));
    assert!(patch.contains("@@ -1 +1 @@\n-old\n+new"));
    assert!(!body.contains("approve once"));
    let decisions = prompt.decision_text().expect("approval decisions");
    assert!(decisions.contains("Yes, and don't ask again for these files"));
    assert!(decisions.contains("[Esc/x] No, and stop"));
}
