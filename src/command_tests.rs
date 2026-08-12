use pretty_assertions::assert_eq;

use super::SlashInput;
use super::matches;
use super::parse;

#[test]
fn parses_command_and_inline_arguments() {
    assert_eq!(
        parse(" /rename useful session "),
        Some(SlashInput {
            name: "rename",
            args: "useful session",
        })
    );
    assert_eq!(parse("ordinary text"), None);
}

#[test]
fn filters_command_menu_by_prefix_until_arguments_start() {
    let names = matches("/re")
        .into_iter()
        .map(|command| command.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["resume", "rename", "review"]);
    assert!(matches("ordinary").is_empty());
    assert!(matches("/review ").is_empty());
}
