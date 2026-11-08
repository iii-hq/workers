//! Offline policy boundary through the public run consumer; no live SDK connection.
use std::sync::{Arc, Mutex};

use memory_consolidate::config::WorkerConfig;
use memory_consolidate::consolidate::{self, ConsolidationTransport, TransportFuture};
use memory_consolidate::functions::{self, Deps, RunRequest};
use serde_json::{json, Value};

fn row(id: &str, text: &str, at: u64) -> Value {
    json!({"id":id,"text":text,"revision":0,"created_at":at,"updated_at":at,
        "stated_at":null,"pinned":false,"corroboration":0,
        "source":{"session_id":"offline-source","entry_id":id}})
}

struct State {
    rows: Vec<Value>,
    calls: Vec<(String, Value)>,
    decisions: usize,
}

struct Boundary {
    state: Mutex<State>,
    reply: Value,
    source_changed: bool,
}

impl Boundary {
    fn new(rows: Vec<Value>, reply: Value) -> Self {
        Self {
            state: Mutex::new(State {
                rows,
                calls: vec![],
                decisions: 0,
            }),
            reply,
            source_changed: false,
        }
    }
}

impl ConsolidationTransport for Boundary {
    fn invoke<'a>(&'a self, function: &'a str, payload: Value) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.calls.push((function.into(), payload.clone()));
            match function {
                "memory::bank::list" => {
                    assert_eq!(payload, json!({}));
                    let live: Vec<_> = state
                        .rows
                        .iter()
                        .filter(|r| r["invalid_at"].is_null())
                        .collect();
                    Ok(json!({"banks":[{"name":"offline","description":"",
                        "memories":live.len(),
                        "pinned":live.iter().filter(|r| r["pinned"] == true).count(),
                        "rules":0}]}))
                }
                "memory::list" => {
                    assert_eq!(payload["bank"], "offline");
                    let live: Vec<_> = state
                        .rows
                        .iter()
                        .filter(|r| r["invalid_at"].is_null())
                        .cloned()
                        .collect();
                    Ok(json!({"total":live.len(),"memories":live}))
                }
                "state::get" => Ok(json!({"value":
                    if payload["key"] == "semantic_cursor:offline" {
                        json!({"updated_at":1,"id":"old"})
                    } else { Value::Null }
                })),
                "state::set" => Ok(json!({})),
                "memory::recall" => Ok(json!({"retrieval":payload["mode"],
                    "memories":state.rows.iter().filter(|r| r["invalid_at"].is_null())
                        .map(|r| json!({"memory":r})).collect::<Vec<_>>()})),
                "memory::get" => {
                    // A provenance-only edit leaves text unchanged; the producer's
                    // joint snapshot guard must still reject the stale action.
                    if self.source_changed {
                        for r in &mut state.rows {
                            if r["id"] == payload["id"] {
                                r["source"]["entry_id"] = json!("changed-entry");
                                r["revision"] = json!(1);
                            }
                        }
                    }
                    Ok(json!({"memory":state.rows.iter()
                        .find(|r| r["id"] == payload["id"]).unwrap()}))
                }
                "memory::supersede" => {
                    let loser = state
                        .rows
                        .iter()
                        .position(|r| r["id"] == payload["id"])
                        .unwrap();
                    let winner = state
                        .rows
                        .iter()
                        .position(|r| r["id"] == payload["superseded_by"])
                        .unwrap();
                    assert_ne!(loser, winner);
                    assert_ne!(state.rows[loser]["pinned"], true);
                    if state.rows[loser] != payload["expected_memory"]
                        || state.rows[winner] != payload["expected_superseded_by"]
                    {
                        return Err("snapshot changed".into());
                    }
                    assert_eq!(payload["reinforce"], true);
                    state.rows[loser]["invalid_at"] = json!(100);
                    state.rows[loser]["superseded_by"] = payload["superseded_by"].clone();
                    Ok(json!({"reinforced":false,"survivor":state.rows[winner]}))
                }
                _ => Err(format!("unexpected offline call: {function}")),
            }
        })
    }

    fn complete<'a>(
        &'a self,
        _model: &'a str,
        _system: &'a str,
        _prompt: String,
        _max_tokens: u32,
    ) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            self.state.lock().unwrap().decisions += 1;
            Ok(json!({"message":{"stop_reason":"stop","content":[
                {"type":"text","text":self.reply.to_string()}]}}))
        })
    }
}

async fn run(boundary: &Boundary, semantic_enabled: bool) -> functions::RunResponse {
    let cfg = WorkerConfig {
        banks: vec!["offline".into()],
        semantic_enabled,
        llm_assist_enabled: false,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    };
    let deps = Deps {
        // Construction does not connect; every effect uses the injected transport.
        iii: Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0")),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
        run_gate: tokio::sync::Mutex::new(()),
    };
    let report = functions::run_with_transport(
        &deps,
        RunRequest {
            bank: None,
            dry_run: Some(false),
        },
        boundary,
    )
    .await
    .unwrap();
    assert_eq!(report.banks.len(), 1, "the offline bank must be exercised");
    let state = boundary.state.lock().unwrap();
    assert!(state
        .calls
        .iter()
        .any(|(f, p)| f == "memory::bank::list" && *p == json!({})));
    assert!(state
        .calls
        .iter()
        .any(|(f, p)| f == "memory::list" && p["bank"] == "offline"));
    report
}

fn proposal(proof: Option<Value>) -> Value {
    let mut action = json!({"type":"merge","survivor":"c1","retired":"n1",
        "reason":"Explicit supplied equivalence decision; not model quality evidence."});
    if let Some(proof) = proof {
        action["proof"] = proof;
    }
    json!({"actions":[action]})
}

fn bilingual() -> Vec<Value> {
    vec![
        row("old", "User prefers tea", 1),
        row("new", "Usuário prefere chá", 2),
    ]
}

#[tokio::test]
async fn deterministic_lane_retains_unproved_translation_in_both_orders() {
    for rows in [
        bilingual(),
        vec![
            row("old", "Usuário prefere chá", 1),
            row("new", "User prefers tea", 2),
        ],
    ] {
        let boundary = Boundary::new(rows.clone(), proposal(None));
        let report = run(&boundary, false).await;
        assert!(report.checkpointed);
        assert_eq!(report.superseded, 0);
        assert!(report.banks[0].groups.is_empty());
        {
            let state = boundary.state.lock().unwrap();
            assert_eq!(state.rows, rows);
            assert_eq!(state.decisions, 0);
            assert!(!state.calls.iter().any(|(f, _)| matches!(
                f.as_str(),
                "memory::recall" | "memory::get" | "memory::supersede"
            )));
        }
        // Turning on the semantic tier alone is not a merge decision either.
        let boundary = Boundary::new(rows.clone(), json!({"actions":[]}));
        let report = run(&boundary, true).await;
        assert_eq!(report.superseded, 0);
        let state = boundary.state.lock().unwrap();
        assert_eq!(state.rows, rows);
        assert_eq!(state.decisions, 1);
    }
}

#[tokio::test]
async fn deterministic_lane_preserves_all_four_positive_retirements() {
    let cases = [
        vec![
            "User prefers concise answers.",
            "user prefers concise answers!",
        ],
        vec!["The API port is 3000", "the api port is 3000."],
        vec![
            "Usuário prefere respostas curtas.",
            "usuário prefere respostas curtas!",
        ],
        vec!["limit < 5", "limit < 5.", "limit > 5"],
    ];
    let mut total = 0;
    for texts in cases {
        let rows: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| row(&format!("record-{i}"), text, i as u64 + 1))
            .collect();
        let boundary = Boundary::new(rows.clone(), json!({"actions":[]}));
        let report = run(&boundary, false).await;
        assert!(report.checkpointed);
        assert_eq!(report.superseded, 1);
        assert_eq!(report.banks[0].detail_proofs.len(), 1);
        assert_eq!(
            report.banks[0].groups.iter().any(|g| g.report_only),
            texts.len() == 3
        );
        let state = boundary.state.lock().unwrap();
        assert_eq!(state.decisions, 0);
        assert_eq!(state.rows[1]["superseded_by"], "record-0");
        for (saved, original) in state.rows.iter().zip(&rows) {
            assert_eq!(saved["text"], original["text"]);
            assert_eq!(saved["source"], original["source"]);
        }
        if texts.len() == 3 {
            assert_eq!(state.rows[2], rows[2]);
        }
        total += report.superseded;
    }
    assert_eq!(total, 4);
}

#[tokio::test]
async fn explicit_certified_bilingual_merge_reaches_consumer_write_boundary() {
    let proof = consolidate::detail_proof("User prefers tea", "Usuário prefere chá").unwrap();
    // Both an explicit proposal with local certification and an explicit supplied
    // certificate use the same consumer path, not a direct validator-only call.
    for supplied in [None, Some(serde_json::to_value(&proof).unwrap())] {
        let rows = bilingual();
        let boundary = Boundary::new(rows.clone(), proposal(supplied));
        let report = run(&boundary, true).await;
        assert!(report.checkpointed);
        assert_eq!(report.superseded, 1);
        assert_eq!(report.banks[0].semantic_rejected, 0);
        assert_eq!(report.banks[0].semantic.len(), 1);
        let action = &report.banks[0].semantic[0];
        assert!(action.applied);
        assert_eq!(action.proof.as_ref(), Some(&proof));
        let state = boundary.state.lock().unwrap();
        assert_eq!(state.decisions, 1);
        assert_eq!(state.rows[1]["superseded_by"], "old");
        let writes: Vec<_> = state
            .calls
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .collect();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1["expected_memory"], rows[1]);
        assert_eq!(writes[0].1["expected_superseded_by"], rows[0]);
    }
}

#[tokio::test]
async fn consumer_rejects_forged_stale_detailed_and_pinned_proposals() {
    let proof = consolidate::detail_proof("User prefers tea", "Usuário prefere chá").unwrap();
    let mut forged = serde_json::to_value(&proof).unwrap();
    forged["retired_atoms"][0]["canonical"] = json!("untrusted");
    let mut stale = serde_json::to_value(&proof).unwrap();
    stale["retired_source"] = json!("Usuário prefere café");
    let mut detail = bilingual();
    detail[1]["text"] = json!("Usuário prefere chá à noite");
    let mut pinned = bilingual();
    pinned[1]["pinned"] = json!(true);
    for (rows, supplied) in [
        (bilingual(), Some(forged)),
        (bilingual(), Some(stale)),
        (detail, None),
        (pinned, Some(serde_json::to_value(&proof).unwrap())),
    ] {
        let boundary = Boundary::new(rows.clone(), proposal(supplied));
        let report = run(&boundary, true).await;
        assert!(report.checkpointed);
        assert_eq!(report.superseded, 0);
        assert_eq!(report.banks[0].semantic_rejected, 1);
        assert!(report.banks[0].semantic.is_empty());
        let state = boundary.state.lock().unwrap();
        assert_eq!(state.decisions, 1);
        assert_eq!(state.rows, rows);
        assert!(!state.calls.iter().any(|(f, _)| f == "memory::supersede"));
    }
}

#[tokio::test]
async fn certified_proposal_preserves_joint_provenance_snapshot_guard() {
    let proof = consolidate::detail_proof("User prefers tea", "Usuário prefere chá").unwrap();
    let mut boundary = Boundary::new(
        bilingual(),
        proposal(Some(serde_json::to_value(proof).unwrap())),
    );
    boundary.source_changed = true;
    let report = run(&boundary, true).await;
    assert!(!report.checkpointed);
    assert_eq!(report.superseded, 0);
    assert_eq!(report.banks[0].errors.len(), 1);
    assert!(report.banks[0].errors[0].contains("snapshot changed"));
    let state = boundary.state.lock().unwrap();
    assert_eq!(state.decisions, 1);
    assert!(state.rows.iter().all(|r| r["invalid_at"].is_null()));
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .count(),
        1
    );
}
