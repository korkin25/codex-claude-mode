use pretty_assertions::assert_eq;

use super::DisplayRoutes;

#[test]
fn routed_turn_uses_child_log_until_completion() {
    let mut routes = DisplayRoutes::default();
    assert_eq!(routes.target("main"), "main");

    routes.begin("main", "child".to_string());
    assert!(routes.is_routed("main"));
    assert_eq!(routes.target("main"), "child");

    routes.end("main");
    assert!(!routes.is_routed("main"));
    assert_eq!(routes.target("main"), "main");
}
