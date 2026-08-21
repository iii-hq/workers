#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

pub fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn update_mode() -> bool {
    std::env::var("UPDATE_GOLDENS").is_ok_and(|value| value == "1")
}

pub fn check_golden(relative_path: &str, actual: &str) -> Result<(), String> {
    let path = golden_root().join(relative_path);
    if update_mode() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        fs::write(&path, actual).map_err(|error| format!("write {}: {error}", path.display()))?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path).map_err(|error| {
        format!(
            "golden file {} unreadable ({error}); run UPDATE_GOLDENS=1 cargo test",
            path.display()
        )
    })?;
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "golden mismatch: tests/golden/{relative_path}; run UPDATE_GOLDENS=1 cargo test"
        ))
    }
}

pub fn assert_typed_schema(label: &str, schema: &schemars::schema::RootSchema) {
    let value = serde_json::to_value(schema).expect("schema serializes");
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{label}: schema is not a JSON object"));
    const DEFINING_KEYWORDS: [&str; 8] = [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
        "items",
    ];
    assert!(
        DEFINING_KEYWORDS
            .iter()
            .any(|keyword| object.contains_key(*keyword)),
        "{label}: schema is permissive or empty: {value}"
    );
}
