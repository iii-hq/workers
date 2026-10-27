//! The replay build and rebuild against a real repository and transcripts
//! (ignored; run by hand):
//!
//! REPLAY_REPO=… REPLAY_DIR=… REPLAY_BASE=<sha> REPLAY_CANDIDATE=<sha> \
//! REPLAY_TRANSCRIPTS=<dir of <run_id>.json> REPLAY_SITES=<sites.json> \
//! cargo test --test replay -- --ignored --nocapture
//!
//! With REPLAY_SAMPLE=<answers per step> (and III_URL, III_NAMESPACE) it
//! also samples every changed step on `router::complete`, as the stage does.

use std::process::{Command, Stdio};

use std::collections::BTreeMap;

use eval::candidate::{replay_binary, CandidateConfigV1};
use eval::contract::ErrorSiteV1;
use futures_util::StreamExt as _;
use iii_sdk::protocol::TriggerRequest;
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

async fn trigger(iii: &iii_sdk::IIIClient, function_id: &str, payload: Value) -> Option<Value> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(180_000),
    })
    .await
    .ok()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real() {
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
    let mut changed = Vec::new();
    for point in &points {
        let input = eval::replay::input(
            point,
            point.cause["model"].as_str().unwrap(),
            point.cause["provider"].as_str().unwrap(),
        );
        let (b, c) = (rebuild(&base, &input), rebuild(&candidate, &input));
        println!(
            "{} step {}{}: exact {:?}, changed {}",
            &point.run_id[..8],
            point.step,
            if point.after { " (after)" } else { "" },
            eval::replay::exact(&b["req"], &point.recorded),
            b != c
        );
        if b != c {
            changed.push((point.clone(), b, c));
        }
    }
    println!("{} of {} steps changed", changed.len(), points.len());
    let Ok(per_step) = std::env::var("REPLAY_SAMPLE") else {
        return;
    };
    let per_step: usize = per_step.parse().unwrap();
    let iii = iii_sdk::register_worker(&env("III_URL"), iii_sdk::InitOptions::default());
    let strip = |mut request: Value| {
        request.as_object_mut().unwrap().remove("req");
        request
    };
    let jobs: Vec<(usize, bool, Value)> = changed
        .iter()
        .enumerate()
        .flat_map(|(at, (_, b, c))| {
            (0..per_step)
                .flat_map(move |_| [(at, false, strip(b.clone())), (at, true, strip(c.clone()))])
        })
        .collect();
    let iii = &iii;
    let answers: Vec<(usize, bool, Value)> = futures_util::stream::iter(jobs)
        .map(|(at, side, request)| async move {
            (at, side, trigger(iii, "router::complete", request).await)
        })
        .buffer_unordered(6)
        .filter_map(|(at, side, answer)| async move { answer.map(|answer| (at, side, answer)) })
        .collect()
        .await;
    let mut targets: Vec<String> = changed
        .iter()
        .flat_map(|(point, _, _)| eval::replay::targets(&point.cause))
        .chain(
            answers
                .iter()
                .flat_map(|(_, _, a)| eval::replay::targets(&a["message"])),
        )
        .collect();
    targets.sort();
    targets.dedup();
    let info = trigger(
        iii,
        "engine::functions::info",
        json!({"function_ids": targets}),
    )
    .await
    .unwrap_or_default();
    let schemas: BTreeMap<String, Value> = info["functions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some((
                f["function_id"].as_str()?.to_string(),
                f["request_schema"].clone(),
            ))
        })
        .collect();
    let mut counts = vec![[(0u32, 0u32); 2]; changed.len()];
    let mut cost = 0.0;
    for (at, side, answer) in &answers {
        let wrong = eval::replay::mistake(&answer["message"], &schemas);
        let slot = &mut counts[*at][usize::from(*side)];
        slot.0 += u32::from(wrong.is_some());
        slot.1 += 1;
        cost += answer["usage"]["cost_usd"].as_f64().unwrap_or(0.0);
    }
    let (mut base_total, mut candidate_total) = ((0, 0), (0, 0));
    for ((point, _, _), [b, c]) in changed.iter().zip(&counts) {
        println!(
            "{} step {}{}: base {}/{} candidate {}/{}",
            &point.run_id[..8],
            point.step,
            if point.after { " (after)" } else { "" },
            b.0,
            b.1,
            c.0,
            c.1
        );
        base_total = (base_total.0 + b.0, base_total.1 + b.1);
        candidate_total = (candidate_total.0 + c.0, candidate_total.1 + c.1);
    }
    let (outcome, reason) = eval::replay::verdict(base_total, candidate_total);
    println!("{outcome:?}: {reason}; cost ${cost:.3}");
}
