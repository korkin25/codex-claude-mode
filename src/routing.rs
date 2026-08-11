use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct DisplayRoutes {
    targets: HashMap<String, String>,
}

impl DisplayRoutes {
    pub(crate) fn begin(&mut self, source_id: &str, target_id: String) {
        self.targets.insert(source_id.to_string(), target_id);
    }

    pub(crate) fn end(&mut self, source_id: &str) {
        self.targets.remove(source_id);
    }

    pub(crate) fn target<'a>(&'a self, source_id: &'a str) -> &'a str {
        self.targets
            .get(source_id)
            .map_or(source_id, String::as_str)
    }

    pub(crate) fn is_routed(&self, source_id: &str) -> bool {
        self.targets.contains_key(source_id)
    }
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
