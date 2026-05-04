//! Smoke tests for the CLI shape table.

use provider_cli::{CliShape, CLI_SHAPES};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_cli::register_with_iii;
}

#[test]
fn cli_shapes_table_is_non_empty() {
    assert!(!CLI_SHAPES.is_empty());
}

#[test]
fn cli_shapes_includes_known_clis() {
    // The downstream registry expects these tags to be discoverable. A
    // future rename would surface here.
    let bins: Vec<&str> = CLI_SHAPES.iter().map(|s: &CliShape| s.bin).collect();
    assert!(bins.contains(&"claude"), "missing claude bin in {bins:?}");
    assert!(bins.contains(&"codex"), "missing codex bin in {bins:?}");
    assert!(
        bins.contains(&"opencode"),
        "missing opencode bin in {bins:?}"
    );
    assert!(bins.contains(&"gemini"), "missing gemini bin in {bins:?}");

    let tags: Vec<&str> = CLI_SHAPES.iter().map(|s: &CliShape| s.tag).collect();
    assert!(tags.contains(&"claude-cli"));
    assert!(tags.contains(&"codex-cli"));
}

#[test]
fn cli_shape_args_are_callable() {
    // Each CliShape exposes a fn pointer that builds a CLI argv from a
    // prompt string. Spot-check that every entry produces non-empty argv.
    for s in CLI_SHAPES {
        let argv = (s.args)("hello");
        assert!(!argv.is_empty(), "{} produced empty argv for prompt", s.tag);
        // The user prompt should always be the final argv entry.
        assert_eq!(argv.last().map(String::as_str), Some("hello"));
    }
}
