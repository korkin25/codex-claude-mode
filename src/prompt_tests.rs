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
    let mut prompt = ServerPrompt::from_request(&message).expect("valid prompt");
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
    let mut prompt = ServerPrompt::from_request(&message).expect("valid prompt");
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
fn control_c_cancels_prompt_and_interrupts_turn() {
    let message = json!({
        "id": 9,
        "method": "item/fileChange/requestApproval",
        "params": {"threadId": "thread", "turnId": "turn", "itemId": "item"}
    });
    let mut prompt = ServerPrompt::from_request(&message).expect("valid prompt");
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
