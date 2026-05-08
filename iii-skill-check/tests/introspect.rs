use std::path::PathBuf;

fn workers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn read_manifest_parses_textstats() {
    let textstats = workers_dir().join("textstats");
    let manifest = iii_skill_check::introspect::read_manifest(&textstats)
        .expect("read_manifest should succeed for textstats");

    assert_eq!(manifest.name, "textstats");
    let desc = manifest
        .description
        .as_ref()
        .expect("textstats has a description");
    assert!(
        desc.starts_with("Text analysis on the iii bus"),
        "unexpected description: {desc}"
    );
}

#[test]
fn read_manifest_errors_when_file_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let result = iii_skill_check::introspect::read_manifest(tmp.path());
    assert!(
        result.is_err(),
        "expected an error when iii.worker.yaml is absent"
    );
}

#[test]
fn read_manifest_errors_on_invalid_yaml() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("iii.worker.yaml"), "not: [valid: yaml").unwrap();
    let result = iii_skill_check::introspect::read_manifest(tmp.path());
    assert!(result.is_err(), "expected a parse error on malformed YAML");
}
