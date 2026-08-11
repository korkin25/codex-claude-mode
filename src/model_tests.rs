use pretty_assertions::assert_eq;
use serde_json::json;

use super::AgentThread;
use super::TokenUsage;

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
        "updatedAt": 42,
        "turns": [{"items": [
            {"type": "userMessage", "id": "u", "content": [{"type": "text", "text": "hello"}]},
            {"type": "agentMessage", "id": "a", "text": "done"}
        ]}]
    });

    let thread = AgentThread::from_json(&value).expect("valid thread");

    assert_eq!(thread.id, "01900000-agent");
    assert_eq!(thread.parent_id.as_deref(), Some("parent"));
    assert_eq!(thread.label, "worker (reviewer)");
    assert_eq!(thread.status, "idle");
    assert_eq!(thread.log, vec!["You: hello", "Assistant: done"]);
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
    thread.complete_item(&json!({"type": "agentMessage", "id": "item", "text": "hello"}));

    assert_eq!(thread.log, vec!["Assistant: hello"]);
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

    assert_eq!(thread.log, vec!["You: normal", "Assistant: normal answer"]);
}
