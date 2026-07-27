#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

pub fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

pub fn check_golden(rel: &str, actual: &str) -> Result<(), String> {
    let path = golden_root().join(rel);
    if std::env::var("UPDATE_GOLDENS").as_deref() == Ok("1") {
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
        Err(format!("golden mismatch: tests/golden/{rel}"))
    }
}

pub fn assert_typed_schema(label: &str, schema: &schemars::schema::RootSchema) {
    let value = serde_json::to_value(schema).expect("schema serializes");
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{label}: schema is not an object"));
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
    assert!(
        DEFINING.iter().any(|key| object.contains_key(*key)),
        "{label}: schema is untyped: {value}"
    );
}
