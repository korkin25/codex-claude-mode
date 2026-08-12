use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use serde_json::json;

use super::ACCENT_CYAN;
use super::Action;
use super::Mode;
use super::PermissionChoice;
use super::SELECTED_BACKGROUND;
use super::SURFACE_BACKGROUND;
use super::SkillBinding;
use super::SkillChoice;
use super::SubmissionInput;
use super::Workspace;
use super::composer_viewport;
use super::wrap_composer_input;
use crate::model::AgentThread;
use crate::prompt::ServerPrompt;
use crate::session::SessionCandidate;

fn buffer_text(buffer: &Buffer) -> String {
    buffer
        .content()
        .iter()
        .fold(String::new(), |mut text, cell| {
            text.push_str(cell.symbol());
            text
        })
}

#[test]
fn session_picker_defaults_to_new_and_can_continue_existing_session() {
    let mut workspace = Workspace::new();
    workspace.show_session_picker(vec![SessionCandidate {
        id: "01900000-existing".to_string(),
        preview: "previous task".to_string(),
        updated_at: 1,
    }]);
    let backend = TestBackend::new(100, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render session picker");
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
    assert!(contents.contains("Choose session"));
    assert!(contents.contains("+ New session"));
    assert!(contents.contains("Continue · previous task"));
    assert_eq!(terminal.backend().buffer()[(4, 4)].symbol(), "┌");
    assert_eq!(
        terminal.backend().buffer()[(4, 4)].style().fg,
        Some(ACCENT_CYAN)
    );
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::SessionSelected(None)
    ));

    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::SessionSelected(Some(id)) if id == "01900000-existing"
    ));
}

#[test]
fn session_picker_blocks_input_while_new_session_is_created() {
    let mut workspace = Workspace::new();
    workspace.show_session_picker(Vec::new());
    workspace.show_session_starting();
    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render starting state");
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

    assert!(contents.contains("Creating a clean Main session"));
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::None
    ));
}

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
fn editing_horizontal_arrows_move_unicode_cursor_without_selecting_agents() {
    let mut workspace = Workspace::new();
    workspace.order = vec!["main".to_string(), "child".to_string()];
    workspace.input = "a界c".to_string();

    workspace.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.selected, 0);
    assert_eq!(workspace.input, "aXc");
}

#[test]
fn editing_page_keys_scroll_log_without_changing_draft_or_mode() {
    let mut workspace = Workspace::new();
    workspace.input = "unfinished draft".to_string();
    workspace.scroll = 50;
    workspace.last_max_scroll = 100;

    workspace.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(workspace.scroll, 40);
    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "unfinished draft");

    workspace.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(workspace.scroll, 50);
    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "unfinished draft");

    workspace.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE));
    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "Xunfinished draftY");
}

#[test]
fn mouse_wheel_scrolls_log_without_leaving_editing_mode() {
    let mut workspace = Workspace::new();
    workspace.input = "unfinished draft".to_string();
    workspace.scroll = 50;
    workspace.last_max_scroll = 100;
    workspace.log_area = Rect::new(0, 0, 80, 20);

    workspace.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(workspace.scroll, 47);
    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "unfinished draft");

    workspace.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(workspace.scroll, 50);
    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "unfinished draft");
}

fn workspace_with_long_main_log(mode: Mode) -> Workspace {
    let mut workspace = Workspace::new();
    let text = (0..40)
        .map(|index| format!("log line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let thread = AgentThread::from_json(&json!({
        "id": "main",
        "parentThreadId": null,
        "status": {"type": "idle"},
        "createdAt": 1,
        "updatedAt": 1,
        "turns": [{"items": [{"type": "agentMessage", "id": "answer", "text": text}]}]
    }))
    .expect("valid main thread");
    workspace.threads.insert(thread.id.clone(), thread);
    workspace.rebuild_tree(Some("main"));
    workspace.mode = mode;
    workspace
}

#[test]
fn page_up_from_bottom_scrolls_long_main_log_on_first_press() {
    for mode in [Mode::Navigation, Mode::Editing] {
        let mut workspace = workspace_with_long_main_log(mode);
        let backend = TestBackend::new(60, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| workspace.render(frame))
            .expect("render long main log");

        assert!(workspace.last_max_scroll > 10);
        let max_scroll = workspace.last_max_scroll;
        assert_eq!(workspace.scroll, u16::MAX);
        workspace.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(workspace.scroll, max_scroll - 10);
        assert_eq!(workspace.mode, mode);
    }
}

#[test]
fn mouse_scroll_up_from_bottom_scrolls_main_log_and_end_keeps_following() {
    let mut workspace = workspace_with_long_main_log(Mode::Navigation);
    let backend = TestBackend::new(60, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render long main log");
    let max_scroll = workspace.last_max_scroll;

    workspace.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: workspace.log_area.x + 1,
        row: workspace.log_area.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(workspace.scroll, max_scroll - 3);
    assert_eq!(workspace.mode, Mode::Navigation);

    workspace.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(workspace.scroll, u16::MAX);
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
fn slash_input_opens_filterable_command_menu_and_inserts_selection() {
    let mut workspace = Workspace::new();
    workspace.input = "/per".to_string();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render commands");
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
    assert!(contents.contains("Commands"));
    assert!(contents.contains("/permissions"));

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        Action::None
    ));
    assert_eq!(workspace.input, "/permissions ");
}

#[test]
fn dollar_input_opens_skill_menu_and_inserts_selection() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![SkillChoice {
        name: "release-notes".to_string(),
        description: "Draft release notes".to_string(),
        path: PathBuf::from("/skills/release-notes/SKILL.md"),
    }]);

    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render skills");
    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("Skills"));
    assert!(contents.contains("$release-notes"));
    assert!(contents.contains("Draft release notes"));

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        Action::None
    ));
    assert_eq!(workspace.input, "$release-notes ");
}

#[test]
fn submission_attaches_only_the_exact_selected_duplicate_skill() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![
        SkillChoice {
            name: "review".to_string(),
            description: "First".to_string(),
            path: PathBuf::from("/skills/first/SKILL.md"),
        },
        SkillChoice {
            name: "review".to_string(),
            description: "Second".to_string(),
            path: PathBuf::from("/skills/second/SKILL.md"),
        },
    ]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let displayed = workspace.input.clone();
    let submission = workspace.build_submission(displayed);

    assert_eq!(
        submission.input,
        vec![
            SubmissionInput::Text("$review ".to_string()),
            SubmissionInput::Skill {
                name: "review".to_string(),
                path: PathBuf::from("/skills/second/SKILL.md"),
            },
        ]
    );
}

#[test]
fn async_image_attachment_closes_and_recomputes_skill_popup_safely() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![SkillChoice {
        name: "review".to_string(),
        description: "Review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    workspace.attach_image(PathBuf::from("/tmp/image.png"), "PNG", 10);

    assert!(workspace.skill_popup.is_none());
    assert!(workspace.input.ends_with("[Image #1 PNG 10 B]"));
}

#[test]
fn image_inserted_before_selected_skill_shifts_its_exact_binding() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![SkillChoice {
        name: "review".to_string(),
        description: "Review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    workspace.attach_image(PathBuf::from("/tmp/image.png"), "PNG", 10);
    let displayed = workspace.input.clone();

    let submission = workspace.build_submission(displayed);

    assert!(submission.input.contains(&SubmissionInput::Skill {
        name: "review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }));
}

#[test]
fn editing_selected_skill_mention_invalidates_its_binding() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![SkillChoice {
        name: "review".to_string(),
        description: "Review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let displayed = workspace.input.clone();

    let submission = workspace.build_submission(displayed);

    assert!(
        !submission
            .input
            .iter()
            .any(|input| matches!(input, SubmissionInput::Skill { .. }))
    );
}

#[test]
fn history_resubmits_exact_duplicate_skill_binding() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![
        SkillChoice {
            name: "review".to_string(),
            description: "First".to_string(),
            path: PathBuf::from("/skills/first/SKILL.md"),
        },
        SkillChoice {
            name: "review".to_string(),
            description: "Second".to_string(),
            path: PathBuf::from("/skills/second/SKILL.md"),
        },
    ]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    workspace.mode = Mode::Editing;

    workspace.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(submission) if submission.input.contains(&SubmissionInput::Skill {
            name: "review".to_string(),
            path: PathBuf::from("/skills/second/SKILL.md"),
        })
    ));
}

#[test]
fn shell_completion_before_skill_binding_shifts_exact_range() {
    let directory = std::env::temp_dir().join(format!("ccm-skill-complete-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("unique-file"), "").unwrap();
    let mut workspace = Workspace::new();
    workspace.completion_cwd = directory.clone();
    workspace.set_skills(vec![SkillChoice {
        name: "review".to_string(),
        description: "Review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    workspace.handle_paste("cat un ".to_string());
    workspace.input_cursor = Some("cat un".len());
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let displayed = workspace.input.clone();

    let submission = workspace.build_submission(displayed);

    assert!(submission.input.contains(&SubmissionInput::Skill {
        name: "review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn whole_input_replacements_clear_stale_skill_bindings() {
    let mut workspace = Workspace::new();
    workspace.set_skills(vec![SkillChoice {
        name: "review".to_string(),
        description: "Review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    }]);
    workspace.handle_key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    workspace.input = "/per".to_string();
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(workspace.skill_bindings.is_empty());

    workspace.skill_bindings.push(SkillBinding {
        start: 0,
        end: 7,
        name: "review".to_string(),
        path: PathBuf::from("/skills/review/SKILL.md"),
    });
    workspace.root_id = Some("main".to_string());
    workspace.order.push("main".to_string());
    let _ = workspace.prepare_subagent_request();
    assert!(workspace.skill_bindings.is_empty());
}

#[test]
fn control_u_clears_the_editing_composer() {
    let mut workspace = Workspace::new();
    workspace.input = "/permissions".to_string();

    workspace.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

    assert!(workspace.input.is_empty());
    assert_eq!(workspace.mode, Mode::Editing);
}

#[test]
fn permission_picker_selects_for_its_captured_thread() {
    let mut workspace = Workspace::new();
    workspace.show_permission_picker(
        "child-thread".to_string(),
        vec![
            PermissionChoice {
                id: ":read-only".to_string(),
                description: "read only".to_string(),
            },
            PermissionChoice {
                id: ":workspace".to_string(),
                description: "workspace".to_string(),
            },
        ],
        Some(":workspace"),
    );

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::PermissionSelected { target_id, profile_id }
            if target_id == "child-thread" && profile_id == ":workspace"
    ));
}

#[test]
fn approval_suspends_and_restores_permission_picker_state() {
    let mut workspace = Workspace::new();
    let thread = AgentThread::from_json(&json!({
        "id": "child-thread",
        "parentThreadId": null,
        "status": {"type": "idle"},
        "createdAt": 1,
        "updatedAt": 1
    }))
    .expect("valid thread");
    workspace.threads.insert(thread.id.clone(), thread);
    workspace.show_permission_picker(
        "child-thread".to_string(),
        vec![
            PermissionChoice {
                id: ":read-only".to_string(),
                description: "read only".to_string(),
            },
            PermissionChoice {
                id: ":workspace".to_string(),
                description: "workspace".to_string(),
            },
        ],
        Some(":read-only"),
    );
    workspace.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    workspace
        .set_prompt(
            ServerPrompt::from_request_with_item(
                &json!({
                    "id": 92,
                    "method": "item/commandExecution/requestApproval",
                    "params": {
                        "threadId": "child-thread",
                        "turnId": "turn",
                        "command": "true",
                        "availableDecisions": ["accept", "cancel"]
                    }
                }),
                None,
            )
            .expect("valid approval"),
        )
        .expect("no existing prompt");

    assert!(workspace.permission_picker.is_none());
    assert!(workspace.suspended_permission_picker.is_some());
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        Action::ResolvePrompt(_)
    ));
    let picker = workspace
        .permission_picker
        .as_ref()
        .expect("picker restored");
    assert_eq!(picker.target_id, "child-thread");
    assert_eq!(picker.selected, 1);
    assert_eq!(picker.choices[picker.selected].id, ":workspace");
}

#[test]
fn rejected_concurrent_prompt_does_not_change_suspended_picker() {
    let mut workspace = Workspace::new();
    let thread = AgentThread::from_json(&json!({
        "id": "main",
        "parentThreadId": null,
        "status": {"type": "idle"},
        "createdAt": 1,
        "updatedAt": 1
    }))
    .expect("valid thread");
    workspace.threads.insert(thread.id.clone(), thread);
    workspace.show_permission_picker(
        "main".to_string(),
        vec![PermissionChoice {
            id: ":workspace".to_string(),
            description: "workspace".to_string(),
        }],
        None,
    );
    let prompt = |id| {
        ServerPrompt::from_request_with_item(
            &json!({
                "id": id,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "main",
                    "turnId": "turn",
                    "command": "true",
                    "availableDecisions": ["accept", "cancel"]
                }
            }),
            None,
        )
        .expect("valid approval")
    };
    workspace
        .set_prompt(prompt(1))
        .expect("first prompt accepted");

    assert!(workspace.set_prompt(prompt(2)).is_err());
    let picker = workspace
        .suspended_permission_picker
        .as_ref()
        .expect("picker remains suspended");
    assert_eq!(picker.target_id, "main");
    assert_eq!(picker.selected, 0);
}

#[test]
fn navigation_i_opens_agent_info_without_editing() {
    let mut workspace = Workspace::new();
    let thread = AgentThread::from_json(&json!({
        "id": "agent-id",
        "sessionId": "session-id",
        "parentThreadId": null,
        "path": "/tmp/rollout.jsonl",
        "cwd": "/tmp/workspace",
        "status": {"type": "idle"},
        "createdAt": 1,
        "updatedAt": 1,
        "turns": []
    }))
    .expect("valid thread");
    workspace.threads.insert(thread.id.clone(), thread);
    workspace.rebuild_tree(Some("agent-id"));
    workspace.set_backend_user_agent("codex-cli/1.2.3");
    workspace.set_codex_versions(Some("1.2.3".to_string()), Some("1.3.0".to_string()));
    workspace.mode = Mode::Navigation;

    workspace.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert!(workspace.info_open);
    assert_eq!(workspace.mode, Mode::Navigation);
    assert!(workspace.input.is_empty());

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render info");
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
    assert!(contents.contains("Session / agent info"));
    assert!(contents.contains("Codex backend: codex-cli/1.2.3"));
    assert!(contents.contains("Codex version: 1.2.3 (latest: 1.3.0)"));
    assert!(contents.contains("Update available"));
    assert!(contents.contains("session-id"));
    assert!(contents.contains("agent-id"));
    assert!(contents.contains("/tmp/rollout.jsonl"));

    workspace.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(workspace.info_scroll, u16::MAX);

    workspace.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!workspace.info_open);
}

#[test]
fn info_overlay_confirms_codex_update_before_returning_action() {
    let mut workspace = Workspace::new();
    let thread = AgentThread::from_json(&json!({
        "id": "main", "parentThreadId": null, "status": {"type": "idle"},
        "createdAt": 1, "updatedAt": 1, "turns": []
    }))
    .expect("valid thread");
    workspace.threads.insert(thread.id.clone(), thread);
    workspace.rebuild_tree(Some("main"));
    workspace.set_codex_versions(Some("1.0.0".to_string()), Some("1.1.0".to_string()));
    workspace.mode = Mode::Navigation;
    workspace.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT)),
        Action::None
    ));
    assert!(workspace.codex_update_confirm);
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        Action::UpdateCodex
    ));
}

#[test]
fn editing_arrows_recall_previous_messages_and_restore_draft() {
    let mut workspace = Workspace::new();
    workspace.input = "first".to_string();
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(text) if text == "first"
    ));
    assert_eq!(workspace.mode, Mode::Navigation);
    workspace.mode = Mode::Editing;
    workspace.input = "second".to_string();
    workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    workspace.mode = Mode::Editing;
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
fn multiline_pastes_are_collapsed_and_expanded_when_submitted() {
    let mut workspace = Workspace::new();
    workspace.input = "before ".to_string();

    workspace.handle_paste("alpha\nbeta\ngamma".to_string());
    workspace.handle_paste("\r\none\r\ntwo".to_string());

    assert_eq!(
        workspace.input,
        "before [Pasted text #1 +2 lines][Pasted text #2 +2 lines]"
    );
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(text) if text == "before alpha\nbeta\ngamma\r\none\r\ntwo"
    ));
    assert_eq!(workspace.mode, Mode::Navigation);
}

#[test]
fn multiline_paste_inserted_in_the_middle_expands_in_display_order() {
    let mut workspace = Workspace::new();
    workspace.input = "left right".to_string();
    for _ in 0..5 {
        workspace.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    }
    workspace.handle_paste("one\ntwo".to_string());
    workspace.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    workspace.handle_paste("three\nfour".to_string());

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(text) if text == "left one\ntworightthree\nfour"
    ));
}

#[test]
fn completion_replaces_only_the_token_before_the_cursor() {
    let directory =
        std::env::temp_dir().join(format!("codex-claude-mode-cursor-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create completion fixture");
    std::fs::write(directory.join("unique-file"), "").expect("write completion fixture");

    let mut workspace = Workspace::new();
    workspace.completion_cwd = directory.clone();
    workspace.input = "cat un tail".to_string();
    workspace.input_cursor = Some("cat un".len());
    workspace.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    assert_eq!(workspace.input, "cat unique-file tail");
    assert_eq!(workspace.actual_input_cursor(), "cat unique-file".len());
    std::fs::remove_dir_all(directory).expect("remove completion fixture");
}

#[test]
fn pasted_text_survives_history_and_an_approval_prompt() {
    let mut workspace = Workspace::new();
    workspace.handle_paste("first\nsecond".to_string());
    let placeholder = workspace.input.clone();
    workspace
        .set_prompt(
            ServerPrompt::from_request_with_item(
                &json!({
                    "id": 91,
                    "method": "item/commandExecution/requestApproval",
                    "params": {
                        "threadId": "main",
                        "turnId": "turn",
                        "command": "true",
                        "availableDecisions": ["accept", "cancel"]
                    }
                }),
                None,
            )
            .expect("valid approval"),
        )
        .expect("no existing prompt");
    workspace.clear_prompt(&json!(91));

    assert_eq!(workspace.input, placeholder);
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(text) if text == "first\nsecond"
    ));
    workspace.mode = Mode::Editing;
    workspace.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(workspace.input, "[Pasted text #1 +1 lines]");
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(text) if text == "first\nsecond"
    ));
}

#[test]
fn multiline_paste_renders_as_a_single_placeholder() {
    let mut workspace = Workspace::new();
    workspace.handle_paste("one\ntwo\nthree".to_string());
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render workspace");

    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("[Pasted text #1 +2 lines]"));
    assert!(!contents.contains("one"));
    assert!(!contents.contains("two"));
}

#[test]
fn alt_i_requests_an_image_paste_without_inserting_text() {
    let mut workspace = Workspace::new();

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT)),
        Action::PasteImage
    ));
    assert!(workspace.input.is_empty());
}

#[test]
fn pasted_image_renders_as_a_chip_and_submits_as_local_image() {
    let mut workspace = Workspace::new();
    workspace.input = "before ".to_string();
    workspace.attach_image(PathBuf::from("/tmp/clipboard.png"), "PNG", 1536);
    workspace.input.push_str(" after");

    assert_eq!(workspace.input, "before [Image #1 PNG 1.5 KB] after");
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Submit(submission)
            if submission.displayed_text == "before [Image #1 PNG 1.5 KB] after"
                && submission.input == vec![
                    SubmissionInput::Text("before ".to_string()),
                    SubmissionInput::LocalImage(PathBuf::from("/tmp/clipboard.png")),
                    SubmissionInput::Text(" after".to_string()),
                ]
    ));
}

#[test]
fn long_composer_input_renders_the_tail_and_keeps_cursor_visible() {
    let mut workspace = Workspace::new();
    workspace.input = format!("{}VISIBLE-TAIL", "x".repeat(60));
    let backend = TestBackend::new(32, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render long composer input");

    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("VISIBLE-TAIL"));
    assert!(!contents.contains("xxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
}

#[test]
fn composer_viewport_is_unicode_width_safe_and_handles_paste_placeholders() {
    assert_eq!(
        composer_viewport("界界界abc", "界界界abc".len(), 6, 1),
        (1, 3, 0)
    );
    assert_eq!(composer_viewport("界界界abc", "界".len(), 6, 1), (0, 2, 0));

    let mut workspace = Workspace::new();
    workspace.handle_paste("one\ntwo\nthree".to_string());
    workspace.input.push_str(&"界".repeat(20));
    workspace.input.push_str("paste-tail");
    workspace.input_cursor = None;
    let backend = TestBackend::new(30, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render paste placeholder tail");

    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("paste-tail"));
    assert!(!contents.contains("[Pasted text #1 +2 lines]"));
}

#[test]
fn composer_preserves_spaces_and_moves_cursor_immediately() {
    assert_eq!(wrap_composer_input("a ", 10), vec!["a "]);
    assert_eq!(composer_viewport("a ", "a ".len(), 10, 1), (0, 2, 0));
    assert_eq!(wrap_composer_input("  a  b ", 20), vec!["  a  b "]);
}

#[test]
fn rendered_cursor_advances_across_spaces_and_unicode_immediately() {
    let mut workspace = Workspace::new();
    workspace.mode = Mode::Editing;
    let backend = TestBackend::new(12, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");

    workspace.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render leading space");
    let leading_cursor = terminal.get_cursor_position().expect("cursor position");
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .cell(leading_cursor)
            .expect("cursor cell")
            .bg,
        ACCENT_CYAN
    );
    workspace.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

    let mut previous = None;

    for character in ['a', ' ', ' ', '界', ' ', 'b', ' ', ' ', ' '] {
        workspace.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        terminal
            .draw(|frame| workspace.render(frame))
            .expect("render composer input");
        let cursor = terminal.get_cursor_position().expect("cursor position");
        let cursor_cell = terminal
            .backend()
            .buffer()
            .cell(cursor)
            .expect("cursor cell");

        assert_eq!(cursor_cell.bg, ACCENT_CYAN, "input: {:?}", workspace.input);
        assert_ne!(Some(cursor), previous, "input: {:?}", workspace.input);
        previous = Some(cursor);
    }

    assert_eq!(workspace.input, "a  界 b   ");
}

#[test]
fn composer_preserves_space_at_wrapped_boundary() {
    assert_eq!(wrap_composer_input("abc ", 4), vec!["abc ", ""]);
    assert_eq!(composer_viewport("abc ", "abc ".len(), 4, 2), (0, 0, 1));
    assert_eq!(wrap_composer_input("abc  d", 4), vec!["abc ", " d"]);
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
    workspace
        .threads
        .get_mut("01900000-main")
        .expect("main thread")
        .push_user_message("highlighted question".to_string());
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
    assert!(contents.contains("You · waiting 00:00"));
    assert!(contents.contains("highlighted question"));
    assert!(!contents.contains("Assistant:"));
    assert!(contents.contains("Message · ↑/↓ history"));
    assert!(contents.contains("Main •"));
    assert!(contents.contains("worker ●"));
    assert!(!contents.contains("0190…main"));
    assert!(!contents.contains("0190…hild"));
    assert!(contents.contains("Activity"));
    assert_eq!(buffer[(1, 18)].style().fg, Some(Color::Reset));
    assert_eq!(buffer[(1, 18)].style().bg, Some(SELECTED_BACKGROUND));
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.style().bg == Some(SURFACE_BACKGROUND))
    );
}

#[test]
fn selected_agent_permission_profile_is_rendered_and_updates_immediately() {
    let mut workspace = Workspace::new();
    for value in [
        json!({"id":"main","parentThreadId":null,"status":{"type":"idle"},"createdAt":1,"updatedAt":1}),
        json!({"id":"child","parentThreadId":"main","agentNickname":"worker","status":{"type":"idle"},"createdAt":2,"updatedAt":2}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }
    workspace.rebuild_tree(Some("main"));
    workspace.set_permission_profile("main", "workspace-write");
    workspace.set_permission_profile("child", "full-access");
    let backend = TestBackend::new(140, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render main permissions");
    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("permissions workspace-write"));
    assert!(!contents.contains("permissions full-access"));

    workspace.selected = 1;
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render child permissions");
    let buffer = terminal.backend().buffer();
    let contents = buffer_text(buffer);
    assert!(contents.contains("permissions full-access"));
    let full_access_cells = buffer
        .content()
        .windows("full-access".len())
        .find(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>() == "full-access")
        .expect("full-access cells");
    assert!(
        full_access_cells
            .iter()
            .all(|cell| cell.style().fg == Some(Color::Red))
    );

    workspace.set_permission_profile("child", "read-only");
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render updated permissions");
    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("permissions read-only"));
    assert!(!contents.contains("permissions full-access"));
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
fn control_d_leaves_editing_then_quits_on_second_press() {
    let mut workspace = Workspace::new();
    workspace.input = "unfinished message".to_string();

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        Action::None
    ));
    assert_eq!(workspace.mode, Mode::Navigation);
    assert_eq!(workspace.input, "unfinished message");
    assert_eq!(workspace.status_line, "Ctrl-D again to quit");
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        Action::Quit
    ));
}

#[test]
fn session_and_subagent_shortcuts_are_direct_and_mnemonic() {
    let mut workspace = Workspace::new();
    for value in [
        json!({"id":"main","parentThreadId":null,"status":{"type":"idle"},"createdAt":1,"updatedAt":1}),
        json!({"id":"child","parentThreadId":"main","agentNickname":"worker","status":{"type":"idle"},"createdAt":2,"updatedAt":2}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }
    workspace.rebuild_tree(Some("main"));
    workspace.selected = 1;

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        Action::SelectionChanged
    ));
    assert_eq!(workspace.selected_id(), Some("main"));
    assert_eq!(workspace.mode, Mode::Editing);
    assert_eq!(workspace.input, "Start a new sub-agent for this task: ");

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
        Action::NewSession
    ));
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        Action::ChooseSession
    ));
}

#[test]
fn approval_prompt_replaces_log_with_explicit_decisions() {
    let mut workspace = Workspace::new();
    workspace.input = "unfinished message to selected agent".to_string();
    workspace
        .set_prompt(
            ServerPrompt::from_request_with_item(
                &json!({
                    "id": 7,
                    "method": "item/commandExecution/requestApproval",
                    "params": {
                        "threadId": "main",
                        "turnId": "turn",
                        "command": "curl https://example.com",
                        "cwd": "/tmp/project",
                        "availableDecisions": ["accept", "cancel"]
                    }
                }),
                None,
            )
            .expect("valid approval"),
        )
        .expect("no existing prompt");
    assert!(workspace.input.is_empty());
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
    assert!(contents.contains("decline and stop"));
    let selected_marker = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .find(|cell| cell.symbol() == "▶")
        .expect("selected decision marker");
    assert_eq!(
        selected_marker.style().bg,
        Some(SELECTED_BACKGROUND),
        "the default decision should have a visible highlight"
    );

    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        Action::ResolvePrompt(_)
    ));
    assert_eq!(workspace.input, "unfinished message to selected agent");
}

#[test]
fn approval_prompt_closes_info_overlay_and_takes_keyboard_focus() {
    let mut workspace = Workspace::new();
    let thread = AgentThread::from_json(&json!({
        "id": "main",
        "parentThreadId": null,
        "status": {"type": "active"},
        "createdAt": 1,
        "updatedAt": 1
    }))
    .expect("valid thread");
    workspace.threads.insert(thread.id.clone(), thread);
    workspace.rebuild_tree(Some("main"));
    workspace.mode = Mode::Navigation;
    workspace.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert!(workspace.info_open);

    workspace
        .set_prompt(
            ServerPrompt::from_request_with_item(
                &json!({
                    "id": 8,
                    "method": "item/commandExecution/requestApproval",
                    "params": {
                        "threadId": "main",
                        "turnId": "turn",
                        "command": "touch approved",
                        "cwd": "/tmp/project",
                        "availableDecisions": ["accept", "cancel"]
                    }
                }),
                None,
            )
            .expect("valid approval"),
        )
        .expect("no existing prompt");

    assert!(!workspace.info_open);
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render approval");
    let contents = buffer_text(terminal.backend().buffer());
    assert!(contents.contains("Approval required"));
    assert!(contents.contains("approve once"));
    assert!(matches!(
        workspace.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        Action::ResolvePrompt(_)
    ));
}

#[test]
fn long_file_approval_keeps_decisions_visible_and_opens_patch_pager() {
    let mut workspace = Workspace::new();
    let diff = (0..80)
        .map(|index| format!("+added line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    workspace
        .set_prompt(
            ServerPrompt::from_request_with_item(
                &json!({
                    "id": 19,
                    "method": "item/fileChange/requestApproval",
                    "params": {
                        "threadId": "child",
                        "turnId": "turn",
                        "itemId": "patch"
                    }
                }),
                Some(&json!({
                    "type": "fileChange",
                    "id": "patch",
                    "changes": [{
                        "path": "src/large.rs",
                        "kind": {"type": "update", "movePath": null},
                        "diff": diff
                    }]
                })),
            )
            .expect("valid file approval"),
        )
        .expect("no existing prompt");
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render approval");
    let approval = buffer_text(terminal.backend().buffer());
    assert!(approval.contains("Would you like to make"));
    assert!(approval.contains("Yes, proceed"));
    assert!(!approval.contains("added line 79"));

    workspace.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render patch pager");
    let patch = buffer_text(terminal.backend().buffer());
    assert!(patch.contains("P A T C H"));
    assert!(patch.contains("added line 0"));

    workspace.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    terminal
        .draw(|frame| workspace.render(frame))
        .expect("render patch end");
    assert!(buffer_text(terminal.backend().buffer()).contains("added line 79"));
    workspace.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(!workspace.patch_open);
    assert!(workspace.prompt.is_some());
}

#[test]
fn tree_hides_closed_subagents_but_keeps_idle_ones() {
    let mut workspace = Workspace::new();
    for value in [
        json!({"id":"main","parentThreadId":null,"status":{"type":"active"},"createdAt":1,"updatedAt":1}),
        json!({"id":"closed","parentThreadId":"main","status":{"type":"closed"},"createdAt":2,"updatedAt":2}),
        json!({"id":"idle","parentThreadId":"main","status":{"type":"idle"},"createdAt":3,"updatedAt":3}),
    ] {
        let thread = AgentThread::from_json(&value).expect("valid thread");
        workspace.threads.insert(thread.id.clone(), thread);
    }

    workspace.rebuild_tree(Some("main"));

    assert_eq!(workspace.order, vec!["main", "idle"]);
    assert!(workspace.threads.contains_key("closed"));
}
