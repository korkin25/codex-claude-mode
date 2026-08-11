use std::collections::HashSet;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionCandidate {
    pub(crate) id: String,
    pub(crate) preview: String,
    pub(crate) updated_at: i64,
}

pub(crate) fn candidates_from_list(result: &Value) -> Vec<SessionCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|thread| thread.get("parentThreadId").is_none_or(Value::is_null))
        .filter_map(|thread| {
            let id = thread.get("id")?.as_str()?.to_string();
            seen.insert(id.clone()).then(|| SessionCandidate {
                id,
                preview: thread
                    .get("preview")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                updated_at: thread
                    .get("updatedAt")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    candidates
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
