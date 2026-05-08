use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn workers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build a minimal worker dir with a rendered README/skill, no leaves.
/// Tests then mutate one specific surface to trigger one specific check.
fn write_minimal_worker(dir: &Path, name: &str) {
    std::fs::write(
        dir.join("iii.worker.yaml"),
        format!(
            "iii: v1\nname: {name}\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\nbin: {name}\ndescription: A small fixture worker for tests.\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("config.yaml"), "# Fixture config.\nkey: value\n").unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(
        dir.join("docs").join("intro.md"),
        "A small fixture worker for tests.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("docs").join("quickstart.md"),
        "```rust\nfn main() {}\n```\n",
    )
    .unwrap();

    let outputs = iii_skill_check::render::render_worker(dir).unwrap();
    std::fs::write(dir.join("README.md"), &outputs.readme).unwrap();
    std::fs::write(dir.join("skill.md"), &outputs.skill).unwrap();
    std::fs::create_dir_all(dir.join("skills")).unwrap();
    for (leaf, body) in &outputs.leaves {
        std::fs::write(dir.join("skills").join(format!("{leaf}.md")), body).unwrap();
    }
}

#[test]
fn textstats_passes_structure_check() {
    let textstats = workers_dir().join("textstats");
    let violations =
        iii_skill_check::structure::check(&textstats).expect("structure check should not error");
    assert!(
        violations.is_empty(),
        "expected zero violations against textstats, got: {violations:?}"
    );
}

#[test]
fn flags_install_mismatch() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    let bad = readme.replace("iii worker add fixture", "iii worker add wrong-name");
    std::fs::write(tmp.path().join("README.md"), bad).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations
            .iter()
            .any(|v| v.message.to_lowercase().contains("install")),
        "expected an install-mismatch violation, got: {violations:?}"
    );
}

#[test]
fn flags_missing_install_section() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let readme = "<!-- generated -->\n\n# fixture\n\nIntro paragraph.\n\n## Quickstart\n\n```rust\nfn main() {}\n```\n\n## Configuration\n\n```yaml\nkey: value\n```\n";
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations.iter().any(|v| v.message.contains("## Install")),
        "expected a missing-section violation, got: {violations:?}"
    );
}

#[test]
fn flags_unbalanced_llm_only() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    readme.push_str("\n<!-- llm-only:start -->\norphan body without close\n");
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations
            .iter()
            .any(|v| v.message.to_lowercase().contains("llm-only")),
        "expected an unbalanced-llm-only violation, got: {violations:?}"
    );
}

#[test]
fn flags_cargo_build_block() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    readme.push_str("\n```bash\ncargo build --release\n```\n");
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations
            .iter()
            .any(|v| v.message.to_lowercase().contains("cargo build")
                || v.message.to_lowercase().contains("source")),
        "expected a source-build violation, got: {violations:?}"
    );
}

#[test]
fn flags_manifest_jq_verification_step() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    readme.push_str("\n```bash\nfixture --manifest | jq\n```\n");
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations.iter().any(|v| v.message.contains("--manifest")),
        "expected --manifest | jq violation, got: {violations:?}"
    );
}

#[test]
fn flags_bin_help_verification_step() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    readme.push_str("\n```bash\nfixture --help\n```\n");
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations.iter().any(|v| v.message.contains("--help")),
        "expected --help violation, got: {violations:?}"
    );
}

#[test]
fn flags_bin_manifest_without_jq() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    readme.push_str("\n```bash\nfixture --manifest\n```\n");
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations.iter().any(|v| v.message.contains("--manifest")),
        "expected --manifest violation, got: {violations:?}"
    );
}

#[test]
fn does_not_flag_help_in_unrelated_context() {
    // The check is bin-name-scoped: `iii --help` in prose or in a different
    // command should not trigger a violation against the fixture worker.
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
    readme.push_str("\nRun `iii --help` to see CLI options.\n");
    std::fs::write(tmp.path().join("README.md"), readme).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        !violations.iter().any(|v| v.message.contains("--help")),
        "should not flag `iii --help` (different bin name), got: {violations:?}"
    );
}

#[test]
fn flags_iii_link_to_unknown_leaf() {
    let tmp = TempDir::new().unwrap();
    write_minimal_worker(tmp.path(), "fixture");
    let mut skill = std::fs::read_to_string(tmp.path().join("skill.md")).unwrap();
    skill.push_str("\n- [bogus](iii://fixture/nonexistent) — broken link\n");
    std::fs::write(tmp.path().join("skill.md"), skill).unwrap();

    let violations = iii_skill_check::structure::check(tmp.path()).unwrap();
    assert!(
        violations.iter().any(|v| v.message.contains("nonexistent")),
        "expected a broken-link violation, got: {violations:?}"
    );
}
