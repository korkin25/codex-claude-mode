use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::Action;
use super::Mode;
use super::Workspace;
use crate::model::AgentThread;

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
