#[test]
fn library_exports_register_entry_point() {
    let _ = &harness::register_with_iii;
}

#[test]
fn expected_workers_matches_yaml_dependency_count() {
    let yaml = include_str!("../iii.worker.yaml");
    let dep_lines = yaml
        .lines()
        .skip_while(|l| !l.starts_with("dependencies:"))
        .skip(1)
        .filter(|l| l.starts_with("  ") && l.contains(':'))
        .count();
    assert_eq!(
        dep_lines,
        harness::EXPECTED_WORKERS.len(),
        "EXPECTED_WORKERS in lib.rs and dependencies in iii.worker.yaml drifted"
    );
}
