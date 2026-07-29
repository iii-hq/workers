use serde_json::json;

use super::*;

#[test]
fn sink_writes_canonical_json_and_registers_relative_path_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut sink = ArtifactSink::new(dir.path());
    let path = sink
        .write_scenario_json("001", "evidence.json", &json!({"z": 1, "a": 2}))
        .unwrap();
    sink.write_scenario_json("001", "evidence.json", &json!({"a": 2, "z": 1}))
        .unwrap();

    assert_eq!(path, "scenarios/001/evidence.json");
    assert_eq!(sink.paths(), ["scenarios/001/evidence.json"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(&path)).unwrap(),
        "{\n  \"a\": 2,\n  \"z\": 1\n}\n"
    );
}

#[test]
fn sink_rejects_paths_outside_the_run_root() {
    let dir = tempfile::tempdir().unwrap();
    let sibling = tempfile::tempdir().unwrap();
    let mut sink = ArtifactSink::new(dir.path());
    assert!(sink.write_json("../escape.json", &json!({})).is_err());
    assert!(sink.write_json("/tmp/escape.json", &json!({})).is_err());
    assert!(write_json(dir.path(), &sibling.path().join("escape.json"), &json!({})).is_err());
}

#[test]
fn run_reports_are_canonical_and_keep_stable_relative_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut sink = ArtifactSink::new(dir.path());
    sink.write_json("result.json", &json!({"z": 1, "a": 2}))
        .unwrap();
    sink.write_json(
        "execution.json",
        &json!({"result_path": "result.json", "duration_ms": 7}),
    )
    .unwrap();

    assert_eq!(sink.paths(), ["result.json", "execution.json"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("result.json")).unwrap(),
        "{\n  \"a\": 2,\n  \"z\": 1\n}\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("execution.json")).unwrap(),
        "{\n  \"duration_ms\": 7,\n  \"result_path\": \"result.json\"\n}\n"
    );
}

#[test]
fn trim_removes_heavyweight_queue_state_and_keeps_reports() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("queue")).unwrap();
    std::fs::write(dir.path().join("queue/queue_store.json"), b"{}").unwrap();
    std::fs::write(dir.path().join("result.json"), b"{}").unwrap();
    std::fs::write(dir.path().join("stack.json"), b"{}").unwrap();

    trim_passing_run(dir.path());

    assert!(!dir.path().join("queue").exists());
    assert!(dir.path().join("result.json").is_file());
    assert!(dir.path().join("stack.json").is_file());
}
