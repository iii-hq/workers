//! GOLDEN FAMILY A — wire-schema snapshots for all 15 `coder::*` functions
//! served by the shell worker.
//!
//! `shell::code::functions::catalog()` is the single source of truth for
//! each function's id, registration description, and schemars-derived
//! request/response schemas (generated with the same
//! `SchemaSettings::draft07()` construction iii-sdk uses at registration,
//! from the same input/output structs). Each entry is serialized to
//! pretty JSON and compared against `tests/golden/schemas/<id>.json`
//! (`::` in the function id maps to `.` in the filename).
//!
//! These snapshots ARE the product surface consumed by LLM agents — any
//! schema, description, or error-shape change must land as an explicit
//! golden diff. Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use shell::code::functions::{catalog, FunctionSpec};

/// `coder::read-file` -> `coder.read-file.json` (no `::` in filenames so
/// the goldens stay portable across filesystems).
fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn spec_to_pretty_json(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "request_schema": spec.request_schema,
        "response_schema": spec.response_schema,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("spec serializes");
    pretty.push('\n');
    pretty
}

/// The catalog must cover exactly the 15 registered functions, in
/// registration order (kept in lockstep with `register_all`).
#[test]
fn catalog_lists_all_fifteen_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "coder::info",
            "coder::context",
            "coder::read-file",
            "coder::search",
            "coder::update-file",
            "coder::apply-patch",
            "coder::worktree-add",
            "coder::worktree-remove",
            "coder::undo",
            "coder::checkpoints",
            "coder::create-file",
            "coder::delete-file",
            "coder::list-folder",
            "coder::tree",
            "coder::move",
        ]
    );
}

/// Every catalog entry matches its committed golden. Mismatches are
/// collected across ALL functions before failing so one run shows the
/// full drift, not just the first file.
#[test]
fn wire_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let rel = golden_file_name(spec.function_id);
        let actual = spec_to_pretty_json(&spec);
        if let Err(msg) = support::check_golden(&rel, &actual) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "{} wire-schema golden(s) drifted:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every catalog entry's request_schema must carry at least one example.
/// New functions must ship a canonical request example — examples are
/// load-bearing wire-contract anchors that ground LLM agent payload shape.
#[test]
fn every_request_schema_has_at_least_one_example() {
    let mut missing = Vec::new();
    for spec in catalog() {
        let has_example = spec
            .request_schema
            .schema
            .metadata
            .as_ref()
            .map(|m| !m.examples.is_empty())
            .unwrap_or(false);
        if !has_example {
            missing.push(spec.function_id);
        }
    }
    assert!(
        missing.is_empty(),
        "request schemas missing at least one example (add \
         #[schemars(example = \"fn_path\")] to the input struct): {missing:?}"
    );
}

// ---------------------------------------------------------------------------
// Example round-trip + key-fact validation.
//
// `from_value` alone is weak: none of the input structs use
// `deny_unknown_fields`, so a typo in an OPTIONAL field deserializes
// silently into the default. Each test below therefore re-asserts the
// example's key facts on the DESERIALIZED struct — pinning every example
// through the real wire deserialization path, not just "it parses".
// ---------------------------------------------------------------------------

/// The examples actually embedded in the wire schema (request_schema
/// metadata) — validating these validates exactly what agents see.
fn request_examples(function_id: &str) -> Vec<serde_json::Value> {
    let spec = catalog()
        .into_iter()
        .find(|s| s.function_id == function_id)
        .unwrap_or_else(|| panic!("no catalog entry for {function_id}"));
    spec.request_schema
        .schema
        .metadata
        .map(|m| m.examples)
        .unwrap_or_default()
}

/// Deserialize example `idx` of `function_id` into the input type, with
/// a readable panic when the example does not round-trip.
fn example_as<T: serde::de::DeserializeOwned>(function_id: &str, idx: usize) -> T {
    let examples = request_examples(function_id);
    let value = examples
        .get(idx)
        .unwrap_or_else(|| panic!("{function_id} has no example at index {idx}"));
    serde_json::from_value(value.clone()).unwrap_or_else(|e| {
        panic!("example {idx} of {function_id} does not deserialize into the input type: {e}")
    })
}

#[test]
fn info_example_round_trips() {
    let _input: shell::code::functions::info::InfoInput = example_as("coder::info", 0);
}

#[test]
fn read_file_single_example_round_trips_with_window() {
    let input: shell::code::functions::read_file::ReadFileInput = example_as("coder::read-file", 0);
    assert_eq!(input.path.as_deref(), Some("src/main.rs"));
    assert_eq!(input.line_from, Some(10));
    assert_eq!(input.line_to, Some(50));
    assert!(
        input.paths.is_none(),
        "single-path example must not set paths"
    );
}

#[test]
fn read_file_batch_example_round_trips_with_both_target_forms() {
    use shell::code::functions::read_file::{ReadFileInput, ReadTarget};
    let input: ReadFileInput = example_as("coder::read-file", 1);
    assert!(input.path.is_none(), "batch example must not set path");
    let targets = input.paths.expect("batch example sets paths");
    assert_eq!(targets.len(), 2);
    assert!(
        matches!(&targets[0], ReadTarget::Path(p) if p == "src/lib.rs"),
        "first target must be the bare-string form"
    );
    assert!(
        matches!(
            &targets[1],
            ReadTarget::Window {
                line_from: Some(1),
                line_to: Some(30),
                ..
            }
        ),
        "second target must be the windowed object form"
    );
}

#[test]
fn search_example_round_trips() {
    let input: shell::code::functions::search::SearchInput = example_as("coder::search", 0);
    assert_eq!(input.query, "fn handle");
    assert_eq!(input.include_globs, vec!["**/*.rs".to_string()]);
    assert_eq!(input.context_lines_before, Some(2));
    assert_eq!(input.context_lines_after, Some(2));
    assert!(
        input.use_default_excludes,
        "example leaves use_default_excludes at its serde default (true)"
    );
    assert!(input.search_content);
    assert!(!input.search_paths);
}

#[test]
fn update_file_example_round_trips_with_four_op_kinds() {
    use shell::code::functions::update_file::{UpdateFileInput, UpdateOp};
    let input: UpdateFileInput = example_as("coder::update-file", 0);
    assert_eq!(input.files.len(), 1);
    let ops = &input.files[0].ops;
    assert_eq!(ops.len(), 4);
    assert!(matches!(
        ops[0],
        UpdateOp::StrReplace {
            replace_all: false,
            ..
        }
    ));
    assert!(matches!(ops[1], UpdateOp::Insert { at_line: 1, .. }));
    assert!(matches!(
        ops[2],
        UpdateOp::UpdateLines {
            from_line: 5,
            to_line: 7,
            ..
        }
    ));
    assert!(matches!(ops[3], UpdateOp::Replace { .. }));
}

#[test]
fn create_file_example_round_trips_with_relative_and_absolute_entries() {
    let input: shell::code::functions::create_file::CreateFileInput =
        example_as("coder::create-file", 0);
    assert_eq!(input.files.len(), 2);
    assert!(
        !input.files[0].path.starts_with('/'),
        "first entry must show the relative-path form"
    );
    assert!(
        input.files[1].path.starts_with("/tmp/"),
        "second entry must show the in-default-root absolute form"
    );
    assert!(!input.files[0].overwrite);
    assert!(input.files[1].overwrite);
}

#[test]
fn delete_file_example_round_trips() {
    let input: shell::code::functions::delete_file::DeleteFileInput =
        example_as("coder::delete-file", 0);
    assert_eq!(input.paths.len(), 2);
    assert!(
        input.recursive,
        "example demonstrates recursive dir removal"
    );
}

#[test]
fn list_folder_example_round_trips() {
    let input: shell::code::functions::list_folder::ListFolderInput =
        example_as("coder::list-folder", 0);
    assert_eq!(input.page_size, Some(50));
}

#[test]
fn tree_example_round_trips() {
    let input: shell::code::functions::tree::TreeInput = example_as("coder::tree", 0);
    assert_eq!(input.max_depth, Some(3));
    assert!(
        input.use_default_excludes,
        "omitting use_default_excludes must default to true (noise exclusion on)"
    );
}

#[test]
fn move_example_round_trips_with_in_root_absolute_destination() {
    let input: shell::code::functions::move_file::MoveFileInput = example_as("coder::move", 0);
    assert_eq!(input.files.len(), 2);
    assert!(!input.files[0].overwrite);
    assert!(input.files[1].overwrite);
    // The absolute destination must live inside a DEFAULT root ("/tmp")
    // — a canonical example must not be rejected with C215 under the
    // default config.
    assert!(
        input.files[1].to.starts_with("/tmp/"),
        "cross-root example destination must be inside the default /tmp root, \
         got: {}",
        input.files[1].to
    );
}

/// No stale goldens: every file under tests/golden/schemas/ must
/// correspond to a current catalog entry (catches renames/removals that
/// forget to delete the old snapshot).
#[test]
fn no_orphan_schema_goldens() {
    let dir = support::golden_root().join("schemas");
    let expected: Vec<String> = catalog()
        .iter()
        .map(|s| format!("{}.json", s.function_id.replace("::", ".")))
        .collect();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // Directory absent only before first UPDATE_GOLDENS run; the
        // snapshot test above already fails loudly in that case.
        Err(_) => return,
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            expected.iter().any(|e| e == &name),
            "orphan golden tests/golden/schemas/{name}: no catalog entry \
             produces it. Delete it or fix the catalog."
        );
    }
}
