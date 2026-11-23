//! Checks on the skills the ade worker ships.

fn skill(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("skills")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `ade::ui-manifest` lists script assets, not page registrations (a page
/// registers itself through `host.pages.register` when its script loads), so
/// the check before reporting a page URL is the script asset, then the page
/// rendering.
#[test]
fn mcp_rules_check_the_page_script_asset_then_the_page() {
    let body = skill("for-mcp-agents.md").replace('\n', " ");
    assert!(
        !body.contains("lists the page before"),
        "asks the manifest for a page id"
    );
    assert!(
        body.contains("`ade::ui-manifest` lists the page's script asset"),
        "{body}"
    );
    assert!(body.contains("mywork/page.js"), "no asset example");
    assert!(body.contains("renders"), "no check that the page renders");
}
