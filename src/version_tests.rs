use super::update_available;
use super::version_from_text;

#[test]
fn extracts_codex_cli_version() {
    assert_eq!(
        version_from_text("codex-cli 1.2.3\n"),
        Some("1.2.3".to_string())
    );
}

#[test]
fn compares_numeric_version_components() {
    assert!(update_available("0.9.9", "0.10.0"));
    assert!(!update_available("1.2.3", "1.2.3"));
    assert!(!update_available("1.2", "1.2.0"));
    assert!(!update_available("2.0.0", "1.99.0"));
}
