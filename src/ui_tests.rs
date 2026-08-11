use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
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
        json!({"id":"child","sessionId":"child","parentThreadId":"main","agentNickname":"child","preview":"","status":{"type":"idle"},"createdAt":2,"updatedAt":2,"turns":[]}),
        json!({"id":"main","sessionId":"main","parentThreadId":null,"preview":"","status":{"type":"idle"},"createdAt":1,"updatedAt":1,"turns":[]}),
        json!({"id":"grandchild","sessionId":"grandchild","parentThreadId":"child","agentNickname":"nested","preview":"","status":{"type":"idle"},"createdAt":3,"updatedAt":3,"turns":[]}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }
    workspace.rebuild_tree(Some("main"));

    assert_eq!(workspace.order, vec!["main", "child", "grandchild"]);
}

#[test]
fn agent_order_does_not_follow_updated_at_changes() {
    let mut workspace = Workspace::new();
    for value in [
        json!({"id":"main","parentThreadId":null,"status":{"type":"idle"},"createdAt":1,"updatedAt":1}),
        json!({"id":"first","parentThreadId":"main","status":{"type":"idle"},"createdAt":10,"updatedAt":100}),
        json!({"id":"second","parentThreadId":"main","status":{"type":"idle"},"createdAt":20,"updatedAt":20}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }
    workspace.rebuild_tree(Some("main"));
    assert_eq!(workspace.order, vec!["main", "first", "second"]);

    workspace
        .threads
        .get_mut("second")
        .expect("second agent")
        .updated_at = 1_000;
    workspace.rebuild_tree(Some("main"));

    assert_eq!(workspace.order, vec!["main", "first", "second"]);
}

#[test]
fn horizontal_arrows_select_agents_in_navigation_mode() {
    let mut workspace = Workspace::new();
    workspace.order = vec!["main".to_string(), "child".to_string()];
    workspace.mode = Mode::Navigation;

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        Action::SelectionChanged
    ));
    assert_eq!(workspace.selected, 1);
    workspace.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(workspace.selected, 0);
}

#[test]
fn printable_key_enters_editing_without_losing_first_character() {
    let mut workspace = Workspace::new();
    workspace.mode = Mode::Navigation;

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        Action::None
    ));

    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "q");
}

#[test]
fn editing_arrows_recall_previous_messages_and_restore_draft() {
    let mut workspace = Workspace::new();
    workspace.input = "first".to_string();
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(text) if text == "first"
    ));
    workspace.input = "second".to_string();
    workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    workspace.input = "draft".to_string();

    workspace.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(workspace.input, "second");
    workspace.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(workspace.input, "first");
    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(workspace.input, "second");
    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(workspace.input, "draft");
}

#[test]
fn workspace_renders_log_above_composer_and_horizontal_agents() {
    let mut workspace = Workspace::new();
    for value in [
        json!({"id":"01900000-main","parentThreadId":null,"preview":"","status":{"type":"idle"},"updatedAt":1,"turns":[{"items":[{"type":"agentMessage","id":"a","text":"main log"}]}]}),
        json!({"id":"01900001-child","parentThreadId":"01900000-main","agentNickname":"worker","preview":"","status":{"type":"active"},"updatedAt":2,"turns":[]}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }
    workspace.rebuild_tree(Some("01900000-main"));
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render workspace");
    let buffer = terminal.backend().buffer();
    let contents = buffer
        .content()
        .iter()
        .fold(String::new(), |mut text, cell| {
            text.push_str(cell.symbol());
            text
        });

    assert_eq!(buffer[(0, 0)].symbol(), "M");
    assert!(contents.contains("main log"));
    assert!(!contents.contains("Assistant:"));
    assert!(contents.contains("Message · ↑/↓ history"));
    assert!(contents.contains("Main • 0190…main"));
    assert!(contents.contains("worker ● 0190…hild"));
    assert!(contents.contains("Activity"));
    assert_eq!(buffer[(1, 18)].style().fg, Some(Color::Reset));
    assert_eq!(buffer[(1, 18)].style().bg, Some(Color::Rgb(42, 50, 56)));
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
