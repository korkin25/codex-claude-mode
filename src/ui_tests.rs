use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

use super::Action;
use super::Mode;
use super::Workspace;
use crate::model::AgentThread;
use crate::prompt::ServerPrompt;

#[test]
fn tree_keeps_main_above_nested_agents() {
    let mut workspace = Workspace::new();
    for value in [
        json!({"id":"child","sessionId":"s","parentThreadId":"main","agentNickname":"child","preview":"","status":{"type":"idle"},"updatedAt":2,"turns":[]}),
        json!({"id":"main","sessionId":"s","parentThreadId":null,"preview":"","status":{"type":"idle"},"updatedAt":1,"turns":[]}),
        json!({"id":"grandchild","sessionId":"s","parentThreadId":"child","agentNickname":"nested","preview":"","status":{"type":"idle"},"updatedAt":3,"turns":[]}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }
    workspace.rebuild_tree(Some("main"));

    assert_eq!(workspace.order, vec!["main", "child", "grandchild"]);
}

#[test]
fn escape_and_enter_switch_modes_without_quitting() {
    let mut workspace = Workspace::new();
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Action::None
    ));
    assert_eq!(workspace.mode, Mode::Navigation);
    workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(workspace.mode, Mode::Editing);
}

#[test]
fn approval_prompt_replaces_log_with_explicit_decisions() {
    let mut workspace = Workspace::new();
    workspace
        .set_prompt(
            ServerPrompt::from_request(&json!({
                "id": 7,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "main",
                    "turnId": "turn",
                    "command": "curl https://example.com",
                    "cwd": "/tmp/project",
                    "availableDecisions": ["accept", "cancel"]
                }
            }))
            .expect("valid approval"),
        )
        .expect("no existing prompt");
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render prompt");
    let contents =
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut text, cell| {
                text.push_str(cell.symbol());
                text
            });

    assert!(contents.contains("Action required"));
    assert!(contents.contains("curl https://example.com"));
    assert!(contents.contains("approve once"));
    assert!(contents.contains("decline and interrupt"));
}
