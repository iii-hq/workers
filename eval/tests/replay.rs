//! The replay build and rebuild against a real repository and transcripts
//! (ignored; run by hand):
//!
//! REPLAY_REPO=… REPLAY_DIR=… REPLAY_BASE=<sha> REPLAY_CANDIDATE=<sha> \
//! REPLAY_TRANSCRIPTS=<dir of <run_id>.json> REPLAY_SITES=<sites.json> \
//! cargo test --test replay -- --ignored --nocapture

use std::process::{Command, Stdio};

use eval::candidate::{replay_binary, CandidateConfigV1};
use eval::contract::ErrorSiteV1;
use serde_json::{json, Value};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn rebuild(binary: &std::path::Path, input: &Value) -> Value {
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.take().unwrap(), input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore]
fn real() {
    let config: CandidateConfigV1 = serde_json::from_value(json!({
        "repository": env("REPLAY_REPO"), "base_sha": env("REPLAY_BASE"),
        "worktrees_dir": env("REPLAY_DIR"), "model": "m", "provider": "p",
        "max_turns": 1, "max_output_tokens": 1, "max_total_tokens": 1, "max_cost_usd": 1.0,
    }))
    .unwrap();
    let base = replay_binary(&config, &env("REPLAY_BASE")).unwrap();
    let candidate = replay_binary(&config, &env("REPLAY_CANDIDATE")).unwrap();
    let sites: Vec<ErrorSiteV1> =
        serde_json::from_slice(&std::fs::read(env("REPLAY_SITES")).unwrap()).unwrap();
    let dir = env("REPLAY_TRANSCRIPTS");
    let runs: Vec<Value> = sites
        .iter()
        .map(|site| site.run_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|run_id| {
            let transcript: Value =
                serde_json::from_slice(&std::fs::read(format!("{dir}/{run_id}.json")).ok()?)
                    .ok()?;
            Some(json!({"run_id": run_id, "transcript": transcript}))
        })
        .collect();
    let scenario = sites[0].scenario_id.clone();
    let bundle = json!({"detail": {"reports": [{"scenario_id": scenario, "available": true,
        "report": {"scenarios": [{"scenario_id": scenario, "runs": runs}]}}]}});
    let points = eval::replay::points(&bundle, &sites);
    let mut changed = 0;
    for point in &points {
        let input = eval::replay::input(
            point,
            point.cause["model"].as_str().unwrap(),
            point.cause["provider"].as_str().unwrap(),
        );
        let (b, c) = (rebuild(&base, &input), rebuild(&candidate, &input));
        changed += usize::from(b != c);
        println!(
            "{} step {}{}: exact {:?}, changed {}",
            &point.run_id[..8],
            point.step,
            if point.after { " (after)" } else { "" },
            eval::replay::exact(&b["req"], &point.recorded),
            b != c
        );
    }
    println!("{changed} of {} steps changed", points.len());
}
