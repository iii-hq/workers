#[test]
fn worker_manifest_matches_the_rust_binary_release_contract() {
    let manifest = include_str!("../iii.worker.yaml");

    for required_line in [
        "name: compose-ui",
        "language: rust",
        "deploy: binary",
        "manifest: Cargo.toml",
        "bin: compose-ui",
    ] {
        assert!(
            manifest.lines().any(|line| line == required_line),
            "missing `{required_line}` from iii.worker.yaml"
        );
    }

    assert!(manifest.contains("runtime:\n  kind: rust"));
}
