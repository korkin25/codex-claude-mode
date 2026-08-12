use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pretty_assertions::assert_eq;
use ratatui::{Terminal, backend::TestBackend};

use super::{BrowserAction, EditorKind, ProjectBrowser};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn navigates_tree_opens_file_and_returns() {
    let root = std::env::temp_dir().join(format!(
        "project-tree-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("README.md"), "hello\n").unwrap();
    let mut browser = ProjectBrowser::open(root.clone());
    browser.handle_key(key(KeyCode::Down));
    browser.handle_key(key(KeyCode::Right));
    browser.handle_key(key(KeyCode::Down));
    browser.handle_key(key(KeyCode::Enter));
    let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
    terminal.draw(|frame| browser.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("fn main()"));
    browser.handle_key(key(KeyCode::Char('q')));
    assert_eq!(
        browser.handle_key(key(KeyCode::Char('q'))),
        BrowserAction::Close
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn editor_action_uses_selected_file() {
    let root = std::env::temp_dir().join(format!(
        "project-tree-editor-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.rs"), "let value = 1;\n").unwrap();
    let mut browser = ProjectBrowser::open(root.clone());
    let selected_file = root.join("a.rs").canonicalize().unwrap();
    browser.handle_key(key(KeyCode::Down));
    assert_eq!(
        browser.handle_key(key(KeyCode::Char('v'))),
        BrowserAction::OpenEditor {
            editor: EditorKind::VsCode,
            path: selected_file
        }
    );
    fs::remove_dir_all(root).unwrap();
}
