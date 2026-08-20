//! Wire-schema snapshots for the `browser::*` Scrapling surface: catalog()
//! vs Python-generated goldens, byte-for-byte.
//!
//! These goldens are read-only (see `support::check_golden_readonly`) — they
//! come from `scripts/gen_goldens.py` running real scrapling 0.4.9, so a pass
//! means the Rust literals still mirror `scrapling/src/schemas.py`. The
//! `browser::` id prefix is the one deliberate divergence; the generator
//! applies it via `wire_id()`.

mod support;

use browser::scrapling::schemas::{catalog, FunctionSpec};

fn spec_to_pretty_json(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "request_schema": spec.request,
        "response_schema": spec.response,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("serializes");
    pretty.push('\n');
    pretty
}

#[test]
fn catalog_lists_all_scraping_functions_at_browser_root() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "browser::fetch",
            "browser::stealthy-fetch",
            "browser::dynamic-fetch",
            "browser::screenshot-url",
            "browser::extract",
            "browser::css",
            "browser::xpath",
            "browser::regex",
            "browser::find-similar",
            "browser::find",
            "browser::find-by-text",
            "browser::find-by-regex",
            "browser::describe",
            "browser::to-markdown",
            "browser::session-open",
            "browser::session-fetch",
            "browser::session-close",
            "browser::session-list",
            "browser::crawl",
        ]
    );
}

#[test]
fn wire_schemas_match_python_goldens_byte_for_byte() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let rel = format!("schemas/{}.json", spec.function_id.replace("::", "."));
        if let Err(msg) = support::check_golden_readonly(&rel, &spec_to_pretty_json(&spec)) {
            failures.push(msg);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_schema_is_typed() {
    for spec in catalog() {
        support::assert_typed_schema_value(&format!("{} request", spec.function_id), &spec.request);
        support::assert_typed_schema_value(
            &format!("{} response", spec.function_id),
            &spec.response,
        );
    }
}

#[test]
fn static_ids_are_catalog_plus_guidance_hook() {
    let mut expected: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    expected.push("browser::inject-guidance");
    assert_eq!(browser::scrapling::STATIC_IDS, expected.as_slice());
}

#[test]
fn registered_wire_ids_do_not_use_the_old_nested_namespace() {
    assert!(browser::scrapling::STATIC_IDS
        .iter()
        .all(|id| !id.starts_with("browser::scrapling::")));
}
