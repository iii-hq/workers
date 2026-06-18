#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

pub fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn update_mode() -> bool {
    std::env::var("UPDATE_GOLDENS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

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
            "golden file {} unreadable ({e}). Run `UPDATE_GOLDENS=1 cargo test`.",
            path.display()
        )
    })?;
    if expected == actual {
        return Ok(());
    }
    Err(format!("golden mismatch: tests/golden/{rel}"))
}

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
        "{label}: schema is the permissive AnyValue/empty schema. Got: {value}"
    );
}
