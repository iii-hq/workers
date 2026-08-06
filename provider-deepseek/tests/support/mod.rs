//! Hand-rolled golden-file harness (deliberately no `insta`/snapshot
//! dependency). Goldens live under `tests/golden/` and are committed;
//! any wire-surface change must show up as an explicit, reviewed diff.
//!
//! Workflow:
//! - `cargo test` compares actual output against the committed goldens.
//! - `UPDATE_GOLDENS=1 cargo test` regenerates the files; review the git
//!   diff, then commit the new goldens alongside the change that caused
//!   them.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

/// Root of the committed golden files.
pub fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn update_mode() -> bool {
    std::env::var("UPDATE_GOLDENS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Compare `actual` against the golden file at `tests/golden/<rel>`.
/// Returns `Err(readable diff hint)` on mismatch or missing golden;
/// with `UPDATE_GOLDENS=1` the file is (re)written and the check passes.
pub fn check_golden(rel: &str, actual: &str) -> Result<(), String> {
    let path = golden_root().join(rel);
    if update_mode() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        fs::write(&path, actual).map_err(|e| format!("write {}: {e}", path.display()))?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path).map_err(|e| {
        format!(
            "golden file {} unreadable ({e}).\n\
             Run `UPDATE_GOLDENS=1 cargo test` to (re)generate, then review \
             and commit the diff.",
            path.display()
        )
    })?;
    if expected == actual {
        return Ok(());
    }
    Err(diff_hint(rel, &expected, actual))
}

/// Readable first-divergence diff hint: line number, expected vs actual
/// around the mismatch, and the regeneration instructions.
fn diff_hint(rel: &str, expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let first_diff = exp_lines
        .iter()
        .zip(act_lines.iter())
        .position(|(e, a)| e != a)
        .unwrap_or_else(|| exp_lines.len().min(act_lines.len()));

    const CONTEXT: usize = 3;
    let lo = first_diff.saturating_sub(CONTEXT);
    let hi = (first_diff + CONTEXT + 1).max(first_diff + 1);

    let mut out = format!(
        "golden mismatch: tests/golden/{rel}\n\
         first divergence at line {} (expected {} lines, actual {} lines)\n",
        first_diff + 1,
        exp_lines.len(),
        act_lines.len()
    );
    out.push_str("--- expected (golden) ---\n");
    for (i, line) in exp_lines.iter().enumerate().skip(lo).take(hi - lo) {
        let marker = if i == first_diff { ">" } else { " " };
        out.push_str(&format!("{marker} {:>4} | {line}\n", i + 1));
    }
    out.push_str("--- actual ---\n");
    for (i, line) in act_lines.iter().enumerate().skip(lo).take(hi - lo) {
        let marker = if i == first_diff { ">" } else { " " };
        out.push_str(&format!("{marker} {:>4} | {line}\n", i + 1));
    }
    out.push_str(
        "If this change is intentional, run `UPDATE_GOLDENS=1 cargo test`, \
         review the git diff, and commit the updated goldens.\n",
    );
    out
}

/// Assert a schemars-derived request/response schema is a *real* schema and
/// not the permissive `AnyValue` schema a `Value` handler emits (the "unknown"
/// schema this whole convention exists to prevent). A real schema carries at
/// least one schema-defining keyword.
pub fn assert_typed_schema(label: &str, schema: &schemars::schema::RootSchema) {
    let value = serde_json::to_value(schema).expect("schema serializes");
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{label}: schema is not a JSON object"));
    const DEFINING: [&str; 8] = [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
        "items",
    ];
    let has_defining = DEFINING.iter().any(|k| obj.contains_key(*k));
    assert!(
        has_defining,
        "{label}: schema is the permissive AnyValue/empty schema (no type/properties/$ref/…). \
         The handler is registered with `Value` — give it a typed struct deriving JsonSchema. \
         Got: {value}"
    );
}
