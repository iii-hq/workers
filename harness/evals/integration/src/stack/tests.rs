use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;

#[test]
fn layout_allocates_the_stable_run_tree() {
    let artifacts = tempfile::tempdir().unwrap();
    let layout = RunLayout::allocate(artifacts.path(), "run-001").unwrap();

    assert_eq!(layout.root, artifacts.path().join("run-001"));
    assert_eq!(layout.engine_dir, layout.root.join("engine"));
    assert_eq!(layout.seeds_dir, layout.root.join("seeds"));
    assert_eq!(layout.logs_dir, layout.root.join("logs"));
    assert_eq!(
        layout.engine_config_path(),
        layout.root.join("engine/engine.yaml")
    );
    assert_eq!(layout.stack_manifest_path(), layout.root.join("stack.json"));
    assert!(layout.engine_dir.is_dir());
    assert!(layout.seeds_dir.is_dir());
    assert!(layout.logs_dir.is_dir());

    let scenario = layout.scenario_dir("C-E2E-001").unwrap();
    assert_eq!(scenario, layout.root.join("scenarios/C-E2E-001"));
    assert!(scenario.is_dir());
}

#[test]
fn layout_rejects_path_traversal_components() {
    let artifacts = tempfile::tempdir().unwrap();
    for unsafe_id in ["", ".", "..", "../escape", "nested/run", "/tmp/escape"] {
        assert!(
            RunLayout::allocate(artifacts.path(), unsafe_id).is_err(),
            "unsafe run id was accepted: {unsafe_id:?}"
        );
    }

    let layout = RunLayout::allocate(artifacts.path(), "safe-run").unwrap();
    assert!(layout.scenario_dir("../escape").is_err());
    assert!(layout.seed_path("../escape").is_err());
    assert!(layout.log_path("../escape", "out").is_err());
}

#[test]
fn manifest_is_canonical_and_uses_layout_paths() {
    let artifacts = tempfile::tempdir().unwrap();
    let layout = RunLayout::allocate(artifacts.path(), "run-001").unwrap();
    let binary = artifacts.path().join("stub-bin");
    std::fs::write(&binary, b"stub").unwrap();
    let bins = StackBins {
        engine: binary.clone(),
        harness: binary.clone(),
        console: None,
        workers: BTreeMap::from([("queue".to_string(), PathBuf::from(&binary))]),
    };

    manifest::write_stack_manifest(&bins, &layout, 3210).unwrap();
    let committed = std::fs::read_to_string(layout.stack_manifest_path()).unwrap();
    let value = manifest::stack_info(&bins, &layout, 3210).unwrap();
    assert_eq!(committed, crate::canonical::canonical_json_pretty(&value));
    assert_eq!(value["profile"], "harness-core-v1");
    assert_eq!(
        value["components"]["controlled_services"],
        serde_json::json!(["scripted-router", "integration-recorder"])
    );
    assert_eq!(value["run_root"], layout.root.to_string_lossy().as_ref());
    assert_eq!(value["port"], 3210);
}

#[test]
fn manifest_fails_when_a_binary_cannot_be_identified() {
    let artifacts = tempfile::tempdir().unwrap();
    let layout = RunLayout::allocate(artifacts.path(), "run-001").unwrap();
    let missing = artifacts.path().join("missing-bin");
    let bins = StackBins {
        engine: missing.clone(),
        harness: missing.clone(),
        console: None,
        workers: BTreeMap::new(),
    };
    let error = manifest::stack_info(&bins, &layout, 3210).unwrap_err();
    assert!(format!("{error:#}").contains("resolve engine binary"));
}

#[tokio::test]
async fn boot_failure_carries_a_complete_typed_teardown() {
    let artifacts = tempfile::tempdir().unwrap();
    let layout = RunLayout::allocate(artifacts.path(), "run-boot-failure").unwrap();
    let bins = StackBins {
        engine: PathBuf::from("/bin/true"),
        harness: PathBuf::from("/bin/true"),
        console: None,
        workers: BTreeMap::new(),
    };

    let failure = match Stack::boot(&bins, layout).await {
        Ok(_) => panic!("missing workers must fail before boot"),
        Err(failure) => failure,
    };
    assert!(failure.teardown.complete());
    assert!(format!("{:#}", failure.error).contains("missing --worker-bin"));
}
