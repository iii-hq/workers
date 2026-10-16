//! Per-session state and step log persistence.
use std::fs;
use std::io;

use context_clm::controller::Ctl;
use context_clm::store::{CachedOut, SessionState, Stats, Store};
use context_clm::types::Msg;
use serde_json::{json, Value};

fn msg(role: &str, text: &str) -> Msg {
    serde_json::from_value(json!({
        "role": role,
        "content": [{"type": "text", "text": text}],
        "timestamp": 1
    }))
    .unwrap()
}

fn populated() -> SessionState {
    SessionState {
        raw0: Some(msg("user", "task")),
        covered: 4,
        prefix_hash: "abc".into(),
        ledger: Some(msg("user", "ledger")),
        proj: vec![msg("assistant", "a"), msg("user", "b")],
        written: Some("[[CTX_TURN 1 role=assistant]]\na".into()),
        ctl: Ctl {
            nudged: vec![0.25],
            finalized: false,
            n_retry: 1,
            consec: 1,
            n_rolled: 2,
            cmds: vec!["ls".into()],
        },
        ratio: 1.25,
        raw_sent: 900,
        edit_mark: 1,
        sent_hashes: vec!["h1".into(), "h2".into()],
        last_step: Some(("t_1".into(), 3)),
        last_out: Some(CachedOut {
            messages: vec![msg("user", "task")],
            system_prompt: "sp".into(),
        }),
        root: Some("/work".into()),
        stopped: false,
        maintenance: true,
        last_usable: 200_000,
        last_usable_model: "anthropic/claude-opus-5-5".into(),
        last_cm_tokens: 1_100,
        last_cm_raw: 900,
        last_cm_tools: 150,
        last_overhead: 512,
        section_limit: 30_720,
        budget: 32_768,
        stats: Stats {
            steps: 3,
            peak_tc: 1200,
            ..Stats::default()
        },
    }
}

#[test]
fn save_load_round_trips_and_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("state"));
    assert_eq!(store.load("s_1").unwrap(), None);
    let mut state = populated();
    store.save("s_1", &state).unwrap();
    assert_eq!(store.load("s_1").unwrap(), Some(state.clone()));
    state.covered = 9;
    state.budget = 16_384;
    store.save("s_1", &state).unwrap();
    let reloaded = store.load("s_1").unwrap().unwrap();
    assert_eq!(reloaded.covered, 9);
    assert_eq!(reloaded.budget, 16_384);
    assert!(!dir.path().join("state/s_1.json.tmp").exists());
}

#[test]
fn partial_state_files_fill_in_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    assert_eq!(SessionState::default().ratio, 1.0);
    assert_eq!(SessionState::default().budget, 0);
    // A state file written before `budget` existed still loads.
    fs::write(dir.path().join("s_1.json"), r#"{"covered":3}"#).unwrap();
    let state = store.load("s_1").unwrap().unwrap();
    assert_eq!(state.covered, 3);
    assert_eq!(state.ratio, 1.0);
    assert_eq!(state.budget, 0);
    assert_eq!(state.ctl, Ctl::default());
}

#[test]
fn a_state_written_by_another_version_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    // A key this version does not know (a newer worker wrote it).
    fs::write(
        dir.path().join("s_1.json"),
        r#"{"covered": 2, "future_field": {"x": 1}}"#,
    )
    .unwrap();
    assert_eq!(store.load("s_1").unwrap().unwrap().covered, 2);
}

#[test]
fn a_partial_cached_request_is_invalid_data() {
    // Never replayed (no system prompt, or no messages): the state is set
    // aside and rebuilt from the window instead.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    for partial in [
        r#"{"covered": 2, "last_out": {"messages": []}}"#,
        r#"{"covered": 2, "last_out": {"system_prompt": "sp"}}"#,
    ] {
        fs::write(dir.path().join("s_1.json"), partial).unwrap();
        assert_eq!(
            store.load("s_1").unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "{partial}"
        );
    }
}

#[test]
fn an_empty_or_truncated_state_is_invalid_data() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    for torn in ["", r#"{"covered": 3, "proj": [{"role": "us"#] {
        fs::write(dir.path().join("s_1.json"), torn).unwrap();
        assert_eq!(
            store.load("s_1").unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "{torn:?}"
        );
    }
}

#[test]
fn corrupt_state_is_an_error_not_a_reset() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    fs::write(dir.path().join("s_1.json"), "{not json").unwrap();
    assert_eq!(
        store.load("s_1").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn sessions_with_lookalike_ids_stay_apart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    store.save("a/b", &populated()).unwrap();
    store.save("a%2Fb", &SessionState::default()).unwrap();
    assert_eq!(store.load("a/b").unwrap(), Some(populated()));
    assert_eq!(store.load("a%2Fb").unwrap(), Some(SessionState::default()));
}

#[test]
fn step_log_appends_lines_and_remove_clears_everything() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    store.save("s_1", &populated()).unwrap();
    store.append_log("s_1", &json!({"step": 1})).unwrap();
    store.append_log("s_1", &json!({"step": 2})).unwrap();
    let log = fs::read_to_string(dir.path().join("log/s_1.jsonl")).unwrap();
    let steps: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(steps, vec![json!({"step": 1}), json!({"step": 2})]);

    store.remove("s_1").unwrap();
    store.remove("s_1").unwrap();
    assert_eq!(store.load("s_1").unwrap(), None);
    assert!(!dir.path().join("log/s_1.jsonl").exists());
}

#[test]
fn a_multi_line_string_stays_one_log_line() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let line = json!({"edit": "line one\nline two\r\n", "step": 1});
    store.append_log("s_1", &line).unwrap();
    let log = fs::read_to_string(dir.path().join("log/s_1.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
    assert_eq!(serde_json::from_str::<Value>(log.trim_end()).unwrap(), line);
}

#[cfg(unix)]
#[test]
fn state_and_log_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // The store creates its own folder, as on a first boot.
    let store = Store::new(dir.path().join("state"));
    store.save("s_1", &populated()).unwrap();
    store.append_log("s_1", &json!({"step": 1})).unwrap();
    let mode = |p: &str| {
        fs::metadata(dir.path().join("state").join(p))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode("s_1.json"), 0o600);
    assert_eq!(mode("log/s_1.jsonl"), 0o600);
    assert_eq!((mode("."), mode("log")), (0o700, 0o700));
}

#[cfg(unix)]
#[test]
fn append_log_never_follows_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let target = dir.path().join("elsewhere");
    fs::create_dir_all(dir.path().join("log")).unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("log/s_1.jsonl")).unwrap();
    assert!(store.append_log("s_1", &json!({"step": 1})).is_err());
    assert!(!target.exists());
}
