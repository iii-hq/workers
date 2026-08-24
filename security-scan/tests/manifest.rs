#[path = "../src/manifest.rs"]
mod manifest;

#[test]
fn manifest_builder_emits_registry_metadata_without_a_binary() {
    let built = manifest::build_manifest();
    let value = serde_json::to_value(&built).expect("serialize worker manifest");
    let _: security_scan::WorkerConfig =
        serde_json::from_value(built.default_config).expect("default_config matches WorkerConfig");

    assert_eq!(value["name"], "security-scan");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["description"], manifest::DESCRIPTION);
    assert_eq!(
        value["default_config"]["repositories"],
        serde_json::json!([])
    );
    assert_eq!(value["default_config"]["analysis"]["model"], "");
    assert_eq!(value["default_config"]["analysis"]["max_turns"], 4);
    assert!(value["supported_targets"]
        .as_array()
        .is_some_and(|targets| targets.len() == 1));
    assert!(value["supported_targets"][0]
        .as_str()
        .is_some_and(|target| !target.is_empty()));
}

#[test]
fn worker_manifest_names_the_same_worker_and_description() {
    let source = include_str!("../iii.worker.yaml");

    assert!(source.lines().any(|line| line == "name: security-scan"));
    assert!(source.lines().any(|line| line == "bin: security-scan"));
    assert!(source.contains(manifest::DESCRIPTION));
    assert!(source.lines().any(|line| line.starts_with("tags: [")));
    assert!(source.lines().any(|line| line == "  github: \"^0.3.0\""));
    assert!(source.lines().any(|line| line == "  harness: \"^1.8.5\""));
    assert!(source.lines().any(|line| line == "  cron: \"^0.21.9\""));
    assert!(source.lines().any(|line| line == "  queue: \"^0.21.5\""));
    assert!(source.lines().any(|line| line == "  worktree: \"^0.3.0\""));
    assert!(source.lines().any(|line| line == "  storage: \"^0.1.0\""));
}

#[test]
fn manifest_subcommand_emits_valid_json_without_connecting_to_iii() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_security-scan"))
        .arg("--manifest")
        .output()
        .expect("spawn security-scan --manifest");
    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("manifest stdout is JSON");
    assert_eq!(manifest["name"], "security-scan");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(manifest["default_config"].is_object());
    assert!(manifest["supported_targets"]
        .as_array()
        .is_some_and(|targets| !targets.is_empty()));
}

#[test]
fn worker_registers_interface_before_dependency_boot() {
    let source = include_str!("../src/main.rs");
    let claim_state = source
        .find(".claim_private_state()")
        .expect("private state claim");

    for registration in [
        "functions::register_all(&iii, &deps)",
        "security_scan::ui::register(&iii)",
        "iii.register_trigger(RegisterTriggerInput",
        "security_scan::schedule::register(&iii",
    ] {
        let position = source.find(registration).expect("interface registration");
        assert!(
            position < claim_state,
            "{registration} must precede dependency boot"
        );
    }
}
