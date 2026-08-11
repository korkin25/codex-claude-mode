use pretty_assertions::assert_eq;
use serde_json::json;

use super::AgentThread;
use super::LogKind;
use super::TokenUsage;

fn log(thread: &AgentThread) -> Vec<(LogKind, &str)> {
    thread
        .log
        .iter()
        .map(|entry| (entry.kind, entry.text.as_str()))
        .collect()
}

#[test]
fn parses_agent_metadata_and_history() {
    let value = json!({
        "id": "01900000-agent",
        "sessionId": "session",
        "parentThreadId": "parent",
        "agentNickname": "worker",
        "agentRole": "reviewer",
        "preview": "inspect",
        "status": {"type": "idle"},
        "canAcceptDirectInput": false,
        "createdAt": 42,
        "updatedAt": 42,
        "turns": [{"startedAt": 41, "items": [
            {"type": "userMessage", "id": "u", "content": [{"type": "text", "text": "hello"}]},
            {"type": "agentMessage", "id": "a", "text": "done"}
        ]}]
    });

    let thread = AgentThread::from_json(&value).expect("valid thread");

    assert_eq!(thread.id, "01900000-agent");
    assert_eq!(thread.parent_id.as_deref(), Some("parent"));
    assert_eq!(thread.label, "worker (reviewer)");
    assert_eq!(thread.status, "idle");
    assert_eq!(log(&thread), Vec::new());
    assert_eq!(thread.tokens, TokenUsage::default());
}

#[test]
fn streaming_item_is_replaced_by_completed_item() {
    let value = json!({
        "id": "agent",
        "sessionId": "session",
        "parentThreadId": null,
        "preview": "",
        "status": {"type": "active", "activeFlags": []},
        "updatedAt": 1,
        "turns": []
    });
    let mut thread = AgentThread::from_json(&value).expect("valid thread");
    thread.append_delta("item", "hel");
    thread.append_delta("item", "lo");
    thread.update_item(
        &json!({"type": "agentMessage", "id": "item", "text": "hello"}),
        true,
    );

    assert_eq!(log(&thread), vec![(LogKind::Agent, "hello")]);
}

#[test]
fn main_history_hides_subagent_transport_turns() {
    let value = json!({
        "id": "main",
        "parentThreadId": null,
        "status": {"type": "idle"},
        "turns": [
            {"items": [
                {"type": "userMessage", "content": [{"type": "text", "text": "normal"}]},
                {"type": "agentMessage", "text": "normal answer"}
            ]},
            {"items": [
                {"type": "userMessage", "content": [{"type": "text", "text": "Пользователь выбрал субагента worker (child). Передай сообщение"}]},
                {"type": "agentMessage", "text": "transport answer"}
            ]}
        ]
    });

    let thread = AgentThread::from_json(&value).expect("valid thread");

    assert_eq!(
        log(&thread),
        vec![(LogKind::User, "normal"), (LogKind::Agent, "normal answer")]
    );
}

#[test]
fn service_items_are_visible_as_activity_and_update_status() {
    let value = json!({
        "id": "agent",
        "parentThreadId": null,
        "status": {"type": "active"},
        "turns": []
    });
    let mut thread = AgentThread::from_json(&value).expect("valid thread");
    for item in [
        json!({"type": "reasoning", "summary": ["private"]}),
        json!({"type": "subAgentActivity", "kind": "interacted", "agentPath": "/root/worker"}),
        json!({"type": "collabAgentToolCall", "tool": "wait", "status": "completed"}),
    ] {
        thread.update_activity(&item);
        thread.update_item(&item, true);
    }

    assert_eq!(
        log(&thread),
        vec![
            (LogKind::Activity, "Thinking: private"),
            (LogKind::Activity, "Sub-agent interacted: /root/worker"),
            (LogKind::Activity, "Agent action [completed]: wait")
        ]
    );
    assert_eq!(thread.display_status(), "working · agent action: wait");
}

#[test]
fn empty_agent_messages_do_not_create_blank_log_rows() {
    let value = json!({
        "id": "agent",
        "parentThreadId": null,
        "status": {"type": "idle"},
        "turns": []
    });
    let mut thread = AgentThread::from_json(&value).expect("valid thread");
    thread.append_delta("item", "");
    thread.update_item(
        &json!({"type": "agentMessage", "id": "item", "text": "  "}),
        true,
    );

    assert_eq!(log(&thread), Vec::new());
}

#[test]
fn user_message_timer_freezes_when_agent_response_starts() {
    let value = json!({
        "id": "agent",
        "parentThreadId": null,
        "status": {"type": "active"},
        "turns": []
    });
    let mut thread = AgentThread::from_json(&value).expect("valid thread");
    thread.push_user_message("question".to_string());
    assert_eq!(
        thread.log[0].timing_label().as_deref(),
        Some("waiting 00:00")
    );

    thread.append_delta("answer", "response");

    assert_eq!(
        thread.log[0].timing_label().as_deref(),
        Some("answered in 00:00")
    );
    assert_eq!(
        log(&thread),
        vec![(LogKind::User, "question"), (LogKind::Agent, "response")]
    );
}

#[test]
fn renders_web_file_and_tool_activity() {
    let value = json!({
        "id": "agent",
        "parentThreadId": null,
        "status": {"type": "active"},
        "turns": []
    });
    let mut thread = AgentThread::from_json(&value).expect("valid thread");
    for item in [
        json!({"type":"webSearch","id":"web","query":"rust tui","action":{"type":"search","query":"rust tui"}}),
        json!({"type":"commandExecution","id":"read","command":"sed -n 1,20p src/main.rs","status":"completed","commandActions":[{"type":"read","path":"src/main.rs"}]}),
        json!({"type":"mcpToolCall","id":"mcp","server":"github","tool":"search","status":"completed"}),
        json!({"type":"fileChange","id":"file","changes":[{"path":"src/main.rs","kind":"update","diff":""}],"status":"completed"}),
    ] {
        thread.update_item(&item, true);
    }

    assert_eq!(
        log(&thread),
        vec![
            (LogKind::Activity, "Web search: rust tui"),
            (LogKind::Activity, "Read [completed]: src/main.rs"),
            (LogKind::Activity, "MCP [completed]: github/search"),
            (LogKind::Activity, "File changes: update: src/main.rs"),
        ]
    );
}

#[test]
fn completed_activity_replaces_live_activity_without_command_output() {
    let value = json!({
        "id": "agent",
        "parentThreadId": null,
        "status": {"type": "active"},
        "turns": []
    });
    let mut thread = AgentThread::from_json(&value).expect("valid thread");
    thread.update_item(
        &json!({
            "type":"commandExecution",
            "id":"read",
            "command":"sed -n 1,20p src/main.rs",
            "status":"inProgress",
            "commandActions":[{"type":"read","path":"src/main.rs"}]
        }),
        false,
    );
    thread.update_item(
        &json!({
            "type":"commandExecution",
            "id":"read",
            "command":"sed -n 1,20p src/main.rs",
            "status":"completed",
            "commandActions":[{"type":"read","path":"src/main.rs"}],
            "aggregatedOutput":"large file contents"
        }),
        true,
    );

    assert_eq!(
        log(&thread),
        vec![(LogKind::Activity, "Read [completed]: src/main.rs")]
    );
}
