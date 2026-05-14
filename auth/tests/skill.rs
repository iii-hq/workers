#[test]
fn skill_starts_with_heading_and_lists_functions() {
    let body = include_str!("../skill.md");
    assert!(body.starts_with("# auth\n"));
    assert!(body.contains("auth::validate"));
    assert!(body.contains("auth::register"));
    assert!(body.contains("auth::token"));
}
