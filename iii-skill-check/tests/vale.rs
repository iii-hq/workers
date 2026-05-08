use std::path::PathBuf;
use tempfile::TempDir;

fn workers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn vale_passes_textstats_artifacts() {
    let textstats = workers_dir().join("textstats");
    let vale_config = workers_dir().join(".vale.ini");

    let readme = textstats.join("README.md");
    let skill = textstats.join("skill.md");
    let analyze = textstats.join("skills").join("analyze.md");
    let diff = textstats.join("skills").join("diff.md");
    let summarize = textstats.join("skills").join("summarize.md");
    let artifacts: Vec<&std::path::Path> = vec![&readme, &skill, &analyze, &diff, &summarize];

    let violations =
        iii_skill_check::vale::run(&artifacts, &vale_config).expect("vale should run");
    assert!(
        violations.is_empty(),
        "expected zero Vale violations on textstats, got: {violations:?}"
    );
}

#[test]
fn vale_flags_marketing_fluff_in_a_readme() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("README.md");
    std::fs::write(
        &path,
        "# Test\n\nBlazing fast and revolutionary.\n",
    )
    .unwrap();

    let vale_config = workers_dir().join(".vale.ini");
    let artifacts: Vec<&std::path::Path> = vec![&path];

    let violations = iii_skill_check::vale::run(&artifacts, &vale_config).unwrap();
    assert!(
        violations
            .iter()
            .any(|v| v.message.to_lowercase().contains("blazing")),
        "expected 'blazing fast' to be flagged, got: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.message.to_lowercase().contains("revolutionary")),
        "expected 'revolutionary' to be flagged, got: {violations:?}"
    );
}

#[test]
fn vale_flags_howto_teaching_phrase_in_skill() {
    let tmp = TempDir::new().unwrap();
    // skill.md is treated as a how-to in .vale.ini — Diataxis.HowTo applies.
    let path = tmp.path().join("skill.md");
    std::fs::write(
        &path,
        "# Test\n\nIn this guide you will learn how to use this worker.\n",
    )
    .unwrap();

    let vale_config = workers_dir().join(".vale.ini");
    let artifacts: Vec<&std::path::Path> = vec![&path];

    let violations = iii_skill_check::vale::run(&artifacts, &vale_config).unwrap();
    assert!(
        violations
            .iter()
            .any(|v| v.message.to_lowercase().contains("teaching")
                || v.message.to_lowercase().contains("how-to")),
        "expected a how-to/teaching violation, got: {violations:?}"
    );
}
