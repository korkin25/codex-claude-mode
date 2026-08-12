#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SlashInput<'a> {
    pub(crate) name: &'a str,
    pub(crate) args: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "new",
        description: "start a new session",
    },
    CommandSpec {
        name: "clear",
        description: "clear and start a new session",
    },
    CommandSpec {
        name: "resume",
        description: "choose a saved session",
    },
    CommandSpec {
        name: "skills",
        description: "list enabled skills",
    },
    CommandSpec {
        name: "status",
        description: "show selected agent status",
    },
    CommandSpec {
        name: "permissions",
        description: "list or select a permission profile",
    },
    CommandSpec {
        name: "agent",
        description: "navigate agents",
    },
    CommandSpec {
        name: "subagents",
        description: "navigate agents",
    },
    CommandSpec {
        name: "compact",
        description: "compact the selected thread",
    },
    CommandSpec {
        name: "rename",
        description: "rename the selected thread",
    },
    CommandSpec {
        name: "fork",
        description: "fork the selected thread",
    },
    CommandSpec {
        name: "archive",
        description: "archive the selected thread",
    },
    CommandSpec {
        name: "delete",
        description: "delete the selected thread",
    },
    CommandSpec {
        name: "review",
        description: "review current changes",
    },
    CommandSpec {
        name: "init",
        description: "create AGENTS.md instructions",
    },
    CommandSpec {
        name: "diff",
        description: "show and explain the git diff",
    },
    CommandSpec {
        name: "quit",
        description: "show how to quit",
    },
    CommandSpec {
        name: "exit",
        description: "show how to quit",
    },
];

pub(crate) fn matches(text: &str) -> Vec<&'static CommandSpec> {
    let Some(prefix) = text.strip_prefix('/') else {
        return Vec::new();
    };
    if prefix.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(prefix))
        .collect()
}

pub(crate) fn parse(text: &str) -> Option<SlashInput<'_>> {
    let text = text.trim();
    let command = text.strip_prefix('/')?;
    let split = command.find(char::is_whitespace).unwrap_or(command.len());
    Some(SlashInput {
        name: &command[..split],
        args: command[split..].trim(),
    })
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
