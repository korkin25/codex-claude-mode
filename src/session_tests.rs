use pretty_assertions::assert_eq;
use serde_json::json;

use super::SessionCandidate;
use super::candidates_from_list;

#[test]
fn extracts_only_root_sessions_newest_first() {
    let result = json!({"data": [
        {"id":"old","parentThreadId":null,"preview":"older work","updatedAt":10},
        {"id":"child","parentThreadId":"old","preview":"worker","updatedAt":30},
        {"id":"new","preview":"newer work","updatedAt":20},
        {"id":"new","preview":"duplicate","updatedAt":20}
    ]});

    assert_eq!(
        candidates_from_list(&result),
        vec![
            SessionCandidate {
                id: "new".to_string(),
                preview: "newer work".to_string(),
                updated_at: 20,
                cwd: String::new(),
            },
            SessionCandidate {
                id: "old".to_string(),
                preview: "older work".to_string(),
                updated_at: 10,
                cwd: String::new(),
            }
        ]
    );
}
