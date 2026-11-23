//! Offline transport-contract tests. Supplied decisions are synthetic;
//! replaying them does not measure a model's semantic quality.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use memory_consolidate::config::WorkerConfig;
use memory_consolidate::consolidate::{self, ConsolidationTransport, MemoryRow, TransportFuture};
use memory_consolidate::functions::{self, Deps, RunRequest};
use memory_consolidate::semantic::{self, SemanticOptions};
use serde_json::{Value, json};

fn corpus() -> Value {
    serde_json::from_str(include_str!("fixtures/consolidation_safety.json")).unwrap()
}

fn row(id: &str, text: &str, at: u64) -> Value {
    json!({"id":id,"text":text,"created_at":at,"updated_at":at,"pinned":false,
        "corroboration":0,"revision":0,"source":{"session_id":"offline-source"}})
}

#[derive(Default)]
struct Snapshot {
    rows: Vec<Value>,
    state: HashMap<String, Value>,
    calls: Vec<(String, Value)>,
    replies: VecDeque<Result<Value, String>>,
    fail_once: Option<String>,
    listed: Option<HashSet<String>>,
    rules: String,
    /// Act as a memory worker that knows the snapshot guards: refuse a stale
    /// `expected_*` and return the reinforced `survivor`. Off = an older
    /// worker that ignores both.
    guarded: bool,
    /// A record edited after every read the pass made (applied on the next
    /// supersede, before its guard check).
    stale: Option<String>,
    /// The bank's `learned` rule as `memory::rule::list` returns it.
    learned: String,
    /// A hand edit appended to `learned` on the next `memory::rule::set`,
    /// before its `expected` check.
    rule_edit: Option<String>,
    /// A hand edit appended to `learned` while the judge call runs.
    judge_edit: Option<String>,
    /// `memory::transcript` lines by the record's `source.session_id`; a
    /// session not listed is `session_not_found`.
    transcripts: HashMap<String, Value>,
    /// Every `memory::transcript` call fails with this error.
    transcript_error: Option<String>,
    /// Live ids with a vector in the query's space. `None`: all of them, and
    /// both recall arms return every live row. `Some`: the lexical arm is
    /// empty and the semantic arm returns only these.
    vectors: Option<HashSet<String>>,
    /// Act as an older memory worker: recall reports no `vector_coverage`.
    no_coverage: bool,
    /// Written to `vectors` on the next `router::complete`: a save that
    /// races in mid-pass.
    judge_vectors: Option<HashSet<String>>,
}

#[derive(Default)]
struct Boundary(Mutex<Snapshot>);

impl Boundary {
    fn with_rows(rows: Vec<Value>) -> Self {
        Self(Mutex::new(Snapshot {
            rows,
            rules: "Permanent rule; preserve bytes.\n".into(),
            state: HashMap::from([
                ("last_run".into(), json!({"ms":7})),
                ("reports".into(), json!([{"previous":true}])),
                (
                    "semantic_cursor:offline".into(),
                    json!({"updated_at":0,"id":""}),
                ),
            ]),
            ..Snapshot::default()
        }))
    }

    fn snapshot(&self) -> Value {
        let inner = self.0.lock().unwrap();
        json!({"rows":inner.rows,"state":inner.state,"rules":inner.rules})
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.0.lock().unwrap().calls.clone()
    }

    fn fail_once(&self, function: &str) {
        self.0.lock().unwrap().fail_once = Some(function.into());
    }

    fn decision(&self, actions: Value) {
        self.0.lock().unwrap().replies.push_back(Ok(json!({
            "message":{"stop_reason":"stop","content":[{"type":"text","text":actions.to_string()}]}
        })));
    }

    fn judge_reply(&self, reply: Value) {
        self.0
            .lock()
            .unwrap()
            .replies
            .push_back(Ok(json!(reply.to_string())));
    }

    fn learned(&self) -> String {
        self.0.lock().unwrap().learned.clone()
    }
}

impl ConsolidationTransport for Boundary {
    fn invoke<'a>(&'a self, function: &'a str, payload: Value) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            let mut inner = self.0.lock().unwrap();
            inner.calls.push((function.into(), payload.clone()));
            let keyed = format!("{function}:{}", payload["key"].as_str().unwrap_or(""));
            if inner.fail_once.as_deref() == Some(function)
                || inner.fail_once.as_deref() == Some(keyed.as_str())
            {
                inner.fail_once = None;
                return Err(format!("{function}: injected transport failure"));
            }
            match function {
                "memory::bank::list" => Ok(json!({"banks":[{"name":"offline"},{"name":"second"}]})),
                "memory::list" => {
                    let live: Vec<Value> = inner
                        .rows
                        .iter()
                        .filter(|r| r["invalid_at"].is_null())
                        .filter(|r| {
                            inner
                                .listed
                                .as_ref()
                                .is_none_or(|ids| ids.contains(r["id"].as_str().unwrap()))
                        })
                        .cloned()
                        .collect();
                    let offset = payload["offset"].as_u64().unwrap() as usize;
                    let limit = payload["limit"].as_u64().unwrap() as usize;
                    let page: Vec<Value> = live.iter().skip(offset).take(limit).cloned().collect();
                    Ok(json!({"memories":page,"total":live.len()}))
                }
                "state::get" => {
                    Ok(json!({"value":inner.state.get(payload["key"].as_str().unwrap())}))
                }
                "state::set" => {
                    inner.state.insert(
                        payload["key"].as_str().unwrap().into(),
                        payload["value"].clone(),
                    );
                    Ok(json!({}))
                }
                "memory::recall" => {
                    let semantic = payload["mode"] == "semantic";
                    let live: Vec<&Value> = inner
                        .rows
                        .iter()
                        .filter(|r| r["invalid_at"].is_null())
                        .collect();
                    let vectored = |r: &&&Value| {
                        inner
                            .vectors
                            .as_ref()
                            .is_none_or(|ids| ids.contains(r["id"].as_str().unwrap()))
                    };
                    let hits: Vec<Value> = live
                        .iter()
                        .filter(|r| (semantic || inner.vectors.is_none()) && vectored(r))
                        .map(|r| json!({"memory":r}))
                        .collect();
                    let mut reply = json!({"retrieval":payload["mode"],"memories":hits});
                    if semantic && !inner.no_coverage {
                        let covered = live.iter().filter(vectored).count();
                        reply["vector_coverage"] = json!({"covered":covered,"live":live.len()});
                    }
                    Ok(reply)
                }
                "memory::get" => inner
                    .rows
                    .iter()
                    .find(|r| r["id"] == payload["id"])
                    .cloned()
                    .map(|m| json!({"memory":m}))
                    .ok_or_else(|| "memory_not_found".into()),
                "memory::supersede" => {
                    if let Some(id) = inner.stale.take() {
                        let edited = inner.rows.iter_mut().find(|r| r["id"] == id).unwrap();
                        edited["revision"] = json!(1);
                        edited["source"]["entry_id"] = json!("edited-meanwhile");
                    }
                    let loser = inner
                        .rows
                        .iter()
                        .position(|r| r["id"] == payload["id"])
                        .unwrap();
                    let winner = inner
                        .rows
                        .iter()
                        .position(|r| r["id"] == payload["superseded_by"])
                        .unwrap();
                    assert_ne!(loser, winner);
                    assert_ne!(inner.rows[loser]["pinned"], true);
                    assert!(inner.rows[loser]["invalid_at"].is_null());
                    if inner.guarded {
                        for (at, expected) in [
                            (loser, &payload["expected_memory"]),
                            (winner, &payload["expected_superseded_by"]),
                        ] {
                            let current = &inner.rows[at];
                            let stale = expected.as_object().is_some_and(|fields| {
                                fields
                                    .iter()
                                    .any(|(k, v)| current.get(k).unwrap_or(&Value::Null) != v)
                            });
                            if stale {
                                return Err(format!(
                                    "memory `{}` changed since expected snapshot",
                                    current["id"]
                                ));
                            }
                        }
                    }
                    inner.rows[loser]["invalid_at"] = json!(100);
                    inner.rows[loser]["superseded_by"] = payload["superseded_by"].clone();
                    let reinforced =
                        payload["reinforce"] == true && inner.rows[winner]["pinned"] != true;
                    if !(inner.guarded && reinforced) {
                        return Ok(json!({"reinforced":reinforced}));
                    }
                    let survivor = &mut inner.rows[winner];
                    survivor["revision"] = json!(survivor["revision"].as_u64().unwrap() + 1);
                    survivor["corroboration"] =
                        json!(survivor["corroboration"].as_u64().unwrap() + 1);
                    Ok(json!({"reinforced":true,"survivor":survivor.clone()}))
                }
                "memory::update" => {
                    if let Some(id) = inner.stale.take() {
                        let edited = inner.rows.iter_mut().find(|r| r["id"] == id).unwrap();
                        edited["revision"] = json!(edited["revision"].as_u64().unwrap() + 1);
                    }
                    let at = inner
                        .rows
                        .iter()
                        .position(|r| r["id"] == payload["id"])
                        .unwrap();
                    let current = &inner.rows[at];
                    assert_ne!(current["pinned"], true, "a pinned memory is never retagged");
                    assert!(current["invalid_at"].is_null());
                    let stale = payload["expected_memory"].as_object().is_some_and(|f| {
                        f.iter()
                            .any(|(k, v)| current.get(k).unwrap_or(&Value::Null) != v)
                    });
                    if inner.guarded && stale {
                        return Err(format!(
                            "memory `{}` changed since expected snapshot",
                            current["id"]
                        ));
                    }
                    let row = &mut inner.rows[at];
                    row["entities"] = payload["entities"].clone();
                    row["revision"] = json!(row["revision"].as_u64().unwrap() + 1);
                    row["updated_at"] = json!(row["updated_at"].as_u64().unwrap() + 1_000);
                    Ok(json!({"memory":row.clone()}))
                }
                "memory::rule::list" => Ok(json!({"bank":payload["bank"],"rules":[
                    {"name":"learned","content":inner.learned,"updated_at":0}
                ]})),
                "memory::rule::set" => {
                    assert_eq!(payload["name"], "learned");
                    if let Some(edit) = inner.rule_edit.take() {
                        inner.learned.push_str(&edit);
                    }
                    let stale = payload["expected"]
                        .as_str()
                        .is_some_and(|e| e != inner.learned);
                    if inner.guarded && stale {
                        return Err("memory::rule::set: invalid_input: rule_changed: rule \
                                    `learned` changed since `expected` was read"
                            .into());
                    }
                    inner.learned = payload["content"].as_str().unwrap().into();
                    Ok(json!({"ok":true,"exists":true}))
                }
                "memory::transcript" => {
                    if let Some(e) = &inner.transcript_error {
                        return Err(e.clone());
                    }
                    let note = inner
                        .rows
                        .iter()
                        .find(|r| r["id"] == payload["id"])
                        .unwrap();
                    let session = note["source"]["session_id"].as_str().unwrap_or("");
                    Ok(match inner.transcripts.get(session) {
                        Some(lines) => json!({"lines":lines}),
                        None => json!({"lines":[],"missing":"session_not_found"}),
                    })
                }
                "memory::correct" => {
                    assert_eq!(payload["expected_memory"]["id"], payload["id"]);
                    let old = inner
                        .rows
                        .iter()
                        .position(|r| r["id"] == payload["id"])
                        .unwrap();
                    let new_id = format!("{}-corrected", payload["id"].as_str().unwrap());
                    let mut fresh = inner.rows[old].clone();
                    fresh["id"] = json!(new_id);
                    fresh["text"] = payload["text"].clone();
                    fresh["updated_at"] = json!(fresh["updated_at"].as_u64().unwrap() + 1_000);
                    fresh["source"]["correction_reason"] = payload["reason"].clone();
                    inner.rows[old]["invalid_at"] = json!(100);
                    inner.rows[old]["superseded_by"] = json!(new_id);
                    let superseded = inner.rows[old].clone();
                    inner.rows.push(fresh.clone());
                    Ok(json!({"corrected":true,"memory":fresh,"superseded":superseded}))
                }
                _ => Err(format!("unexpected offline RPC: {function}")),
            }
        })
    }

    fn judge<'a>(
        &'a self,
        _model: &'a str,
        prompt: String,
    ) -> TransportFuture<'a, (memory_consolidate::llm::JudgeReply, Value)> {
        Box::pin(async move {
            let mut inner = self.0.lock().unwrap();
            inner
                .calls
                .push(("offline::reorder-decision".into(), json!({"prompt":prompt})));
            if let Some(edit) = inner.judge_edit.take() {
                inner.learned.push_str(&edit);
            }
            let reply = inner
                .replies
                .pop_front()
                .unwrap_or_else(|| Err("no synthetic reorder decision supplied".into()))?;
            Ok((
                memory_consolidate::llm::parse_reply(reply.as_str().unwrap()),
                json!({"input":50,"cache_read":10,"output":5,"cost_usd":0.001}),
            ))
        })
    }

    fn complete<'a>(
        &'a self,
        _model: &'a str,
        _system: &'a str,
        prompt: String,
        _max_tokens: u32,
    ) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            let mut inner = self.0.lock().unwrap();
            inner
                .calls
                .push(("router::complete".into(), json!({"prompt":prompt})));
            if let Some(ids) = inner.judge_vectors.take() {
                inner.vectors = Some(ids);
            }
            inner
                .replies
                .pop_front()
                .unwrap_or_else(|| Err("no synthetic decision supplied".into()))
        })
    }
}

fn supersedes(boundary: &Boundary) -> Vec<Value> {
    boundary
        .calls()
        .into_iter()
        .filter(|(f, _)| f == "memory::supersede")
        .map(|(_, p)| p)
        .collect()
}

/// The retired id of a supersede that carries both expected snapshots.
fn guarded_id(payload: &Value) -> Value {
    assert_eq!(payload["expected_memory"]["id"], payload["id"]);
    assert_eq!(
        payload["expected_superseded_by"]["id"],
        payload["superseded_by"]
    );
    payload["id"].clone()
}

fn deterministic_only() -> WorkerConfig {
    WorkerConfig {
        semantic_enabled: false,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    }
}

fn semantic_only() -> WorkerConfig {
    WorkerConfig {
        semantic_enabled: true,
        semantic_batch: 1,
        semantic_candidates: 1,
        semantic_max_per_run: 1,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    }
}

fn deps(cfg: WorkerConfig) -> Deps {
    // new() never connects. The handler's injected transport is the only IO.
    Deps {
        iii: Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0")),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
        run_gate: tokio::sync::Mutex::new(()),
    }
}

async fn run(boundary: &Boundary, mut cfg: WorkerConfig, dry_run: bool) -> functions::RunResponse {
    if cfg.banks.is_empty() {
        cfg.banks = vec!["offline".into()];
    }
    functions::run_with_transport(
        &deps(cfg),
        RunRequest {
            bank: None,
            dry_run: Some(dry_run),
        },
        boundary,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn dry_run_handler_is_read_only_and_never_requests_decisions_or_embeddings() {
    let boundary = Boundary::with_rows(vec![
        row("a", "Keep concise answers.", 1),
        row("b", "keep concise answers!", 2),
    ]);
    let before = boundary.snapshot();
    let cfg = WorkerConfig {
        llm_assist_enabled: true,
        semantic_enabled: true,
        ..WorkerConfig::default()
    };
    let response = run(&boundary, cfg.clone(), true).await;
    let again = run(&boundary, cfg, true).await;
    assert!(response.dry_run);
    assert!(!response.checkpointed);
    assert_eq!(response.superseded, 0);
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::to_value(&again).unwrap()
    );
    assert_eq!(response.banks[0].groups[0].loser_ids, ["b"]);
    assert!(response.banks[0].semantic.is_empty());
    assert!(
        response.banks[0]
            .notes
            .iter()
            .any(|n| n.contains("unevaluated"))
    );
    assert_eq!(
        before,
        boundary.snapshot(),
        "all records, cursor, rules and schedule must be unchanged"
    );
    let calls = boundary.calls();
    assert_eq!(
        calls.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>(),
        [
            "memory::bank::list",
            "memory::list",
            "memory::bank::list",
            "memory::list"
        ]
    );
}

#[tokio::test]
async fn configured_dry_run_and_rejected_bank_preserve_state() {
    let boundary = Boundary::with_rows(vec![row("a", "Same fact", 1), row("b", "same fact!", 2)]);
    let cfg = WorkerConfig {
        dry_run: true,
        banks: vec!["offline".into()],
        ..WorkerConfig::default()
    };
    let deps = deps(cfg);
    let before = boundary.snapshot();
    let denied = functions::run_with_transport(
        &deps,
        RunRequest {
            bank: Some("outside".into()),
            dry_run: None,
        },
        &boundary,
    )
    .await;
    assert!(denied.is_err());
    assert!(boundary.calls().is_empty());
    let response = functions::run_with_transport(
        &deps,
        RunRequest {
            bank: Some("offline".into()),
            dry_run: None,
        },
        &boundary,
    )
    .await
    .unwrap();
    // Planning mode still records its plan and checkpoints, so the schedule
    // waits out the interval instead of rescanning every tick.
    assert!(response.dry_run);
    assert!(response.checkpointed);
    assert_eq!(response.superseded, 0);
    let after = boundary.snapshot();
    assert_eq!(before["rows"], after["rows"]);
    assert_eq!(before["rules"], after["rules"]);
    assert_eq!(
        before["state"]["semantic_cursor:offline"],
        after["state"]["semantic_cursor:offline"]
    );
    let reports = &after["state"]["reports"];
    assert_eq!(reports[0]["dry_run"], true);
    assert_eq!(
        reports[0]["banks"][0]["groups"][0]["loser_ids"],
        json!(["b"])
    );
    assert!(reports[0]["ms"].is_u64());
    assert_eq!(reports[1], json!({"previous":true}), "older reports kept");
    assert_ne!(before["state"]["last_run"], after["state"]["last_run"]);
    assert_eq!(
        boundary
            .calls()
            .iter()
            .map(|(f, p)| format!("{f} {}", p["key"].as_str().unwrap_or("")))
            .collect::<Vec<_>>(),
        [
            "memory::list ",
            "state::get reports",
            "state::set reports",
            "state::set last_run"
        ]
    );
}

#[tokio::test]
async fn deterministic_corpus_requires_positive_merges_and_zero_harmful_calls() {
    let mut positives = 0;
    let mut harmful = Vec::new();
    for case in corpus()["deterministic"].as_array().unwrap() {
        let rows: Vec<Value> = case["texts"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, text)| {
                row(
                    &char::from(b'a' + i as u8).to_string(),
                    text.as_str().unwrap(),
                    i as u64 + 1,
                )
            })
            .collect();
        let boundary = Boundary::with_rows(rows.clone());
        let cfg = WorkerConfig {
            semantic_enabled: false,
            promote_corroboration_threshold: 0,
            ..WorkerConfig::default()
        };
        let response = run(&boundary, cfg, false).await;
        assert!(
            response.checkpointed,
            "{}: {:?}",
            case["name"], response.banks[0].errors
        );
        let actual: Vec<Value> = supersedes(&boundary).iter().map(guarded_id).collect();
        let expected = case["retire"].as_array().unwrap();
        for id in &actual {
            if !expected.contains(id) {
                harmful.push(json!({"case":case["name"],"id":id}));
            }
        }
        assert_eq!(&actual, expected, "{}", case["name"]);
        assert_eq!(response.superseded, expected.len());
        positives += actual.len();
        assert_eq!(
            response.banks[0].groups.iter().any(|g| g.report_only),
            case["review"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        let after = boundary.snapshot();
        assert_eq!(after["rules"], "Permanent rule; preserve bytes.\n");
        for original in rows {
            let saved = after["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == original["id"])
                .unwrap();
            assert_eq!(saved["text"], original["text"]);
            assert_eq!(saved["source"], original["source"]);
            if expected.contains(&original["id"]) {
                assert_eq!(saved["superseded_by"], "a");
            } else {
                assert!(saved["invalid_at"].is_null());
            }
        }
    }
    assert_eq!(positives, 4, "keep-all is not a passing implementation");
    assert!(harmful.is_empty(), "harmful calls: {harmful:?}");
}

#[tokio::test]
async fn pinned_legacy_and_cap_use_the_same_handler_without_rule_writes() {
    let mut pinned = row("pin", "Same fact", 5);
    pinned["pinned"] = json!(true);
    let boundary = Boundary::with_rows(vec![
        row("a", "same fact!", 1),
        row("b", "SAME fact", 2),
        pinned.clone(),
    ]);
    let cfg = WorkerConfig {
        max_supersedes_per_run: 1,
        semantic_enabled: false,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    };
    let response = run(&boundary, cfg.clone(), false).await;
    assert_eq!(response.superseded, 1);
    assert_eq!(
        response.banks[0].reinforced, 0,
        "pinned winner is not reinforced"
    );
    assert_eq!(response.banks[0].capped, 1);
    assert_eq!(boundary.snapshot()["rows"][2], pinned);
    assert_eq!(
        boundary
            .calls()
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .count(),
        1
    );
    let retry = run(&boundary, cfg, false).await;
    assert_eq!(retry.superseded, 1);
    assert_eq!(retry.banks[0].capped, 0);
    assert_eq!(boundary.snapshot()["rows"][2], pinned);
    assert!(
        boundary
            .calls()
            .iter()
            .all(|(f, _)| !f.starts_with("memory::rule::"))
    );
}

#[tokio::test]
async fn failed_mutation_holds_checkpoint_and_retry_only_applies_live_duplicates() {
    let boundary = Boundary::with_rows(vec![row("a", "same fact", 1), row("b", "Same fact!", 2)]);
    boundary.fail_once("memory::supersede");
    let cfg = WorkerConfig {
        semantic_enabled: false,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    };
    let before = boundary.snapshot();
    let response = run(&boundary, cfg.clone(), false).await;
    assert!(!response.checkpointed);
    assert_eq!(response.superseded, 0);
    assert!(!response.banks[0].errors.is_empty());
    assert_eq!(before["rows"], boundary.snapshot()["rows"]);
    assert_eq!(
        before["state"]["last_run"],
        boundary.snapshot()["state"]["last_run"]
    );
    assert!(
        !boundary
            .calls()
            .iter()
            .any(|(f, p)| f == "state::set" && p["key"] == "last_run")
    );
    let retry = run(&boundary, cfg.clone(), false).await;
    assert!(retry.checkpointed);
    assert_eq!(retry.superseded, 1);
    assert_eq!(run(&boundary, cfg, false).await.superseded, 0);
}

#[tokio::test]
async fn checkpoint_write_failure_is_reported_without_reapplying_retired_rows() {
    let boundary = Boundary::with_rows(vec![row("a", "same fact", 1), row("b", "Same fact!", 2)]);
    let cfg = WorkerConfig {
        semantic_enabled: false,
        ..WorkerConfig::default()
    };
    boundary.fail_once("state::set:last_run");
    let response = run(&boundary, cfg.clone(), false).await;
    assert!(!response.checkpointed);
    assert_eq!(
        response.superseded, 1,
        "Memory may already have applied a write"
    );
    assert_eq!(boundary.snapshot()["state"]["last_run"], json!({"ms":7}));
    let retry = run(&boundary, cfg, false).await;
    assert!(retry.checkpointed);
    assert_eq!(
        retry.superseded, 0,
        "retry must not send the retired row again"
    );
    assert_eq!(
        boundary
            .calls()
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .count(),
        1
    );
}

#[tokio::test]
async fn supplied_semantic_corpus_runs_through_handler_and_checks_actual_mutation_calls() {
    let fixture = corpus();
    let mut positives = 0;
    for case in fixture["semantic_inputs"].as_array().unwrap() {
        let boundary =
            Boundary::with_rows(vec![case["pending"].clone(), case["candidate"].clone()]);
        // Only the pending record is newly scanned. Recall still exposes
        // the stored candidate, including legacy statement timestamps.
        boundary.0.lock().unwrap().listed = Some(HashSet::from([case["pending"]["id"]
            .as_str()
            .unwrap()
            .to_string()]));
        boundary.decision(case["reply"].clone());
        let before = boundary.snapshot();
        let response = run(&boundary, semantic_only(), false).await;
        let expected = &fixture["semantic_expected"][case["name"].as_str().unwrap()];
        let actual: Vec<Value> = supersedes(&boundary).iter().map(guarded_id).collect();
        assert_eq!(
            actual,
            expected["retire"].as_array().unwrap().clone(),
            "{}",
            case["name"]
        );
        assert_eq!(
            response.banks[0].semantic_rejected,
            expected["rejected"].as_u64().unwrap() as usize,
            "{}",
            case["name"]
        );
        assert_eq!(response.superseded, actual.len());
        let after = boundary.snapshot();
        assert_eq!(before["rules"], after["rules"]);
        for (before, after) in before["rows"]
            .as_array()
            .unwrap()
            .iter()
            .zip(after["rows"].as_array().unwrap())
        {
            assert_eq!(before["text"], after["text"]);
            if !actual.contains(&before["id"]) {
                assert_eq!(before, after);
            }
        }
        assert!(
            boundary
                .calls()
                .iter()
                .all(|(f, _)| f != "router::models::list"
                    && f != "router::provider::list"
                    && f != "router::embed"
                    && !f.starts_with("memory::rule::"))
        );
        positives += actual.len();
    }
    assert_eq!(
        positives, 2,
        "supplied positive decisions must reach the mutation seam"
    );
}

#[test]
fn supplied_semantic_decisions_are_validated_separately_from_expected_labels() {
    let fixture = corpus();
    let mut positives = 0;
    for case in fixture["semantic_inputs"].as_array().unwrap() {
        let pending: MemoryRow = serde_json::from_value(case["pending"].clone()).unwrap();
        let candidate: MemoryRow = serde_json::from_value(case["candidate"].clone()).unwrap();
        let batch = semantic::build_batch(&[(pending, vec![candidate])]);
        let raw = semantic::parse_actions(&case["reply"].to_string()).unwrap();
        let verdict = semantic::validate(&raw, &batch, &HashSet::new(), 1);
        let expected = &fixture["semantic_expected"][case["name"].as_str().unwrap()];
        let actual: Vec<Value> = verdict
            .accepted
            .iter()
            .map(|a| json!(a.retire.id))
            .collect();
        assert_eq!(
            actual,
            expected["retire"].as_array().unwrap().clone(),
            "{}: {:?}",
            case["name"],
            verdict.rejected
        );
        assert_eq!(
            verdict.rejected.len(),
            expected["rejected"].as_u64().unwrap() as usize,
            "{}",
            case["name"]
        );
        positives += actual.len();
    }
    assert_eq!(positives, 2);
}

#[tokio::test]
async fn failed_scan_and_dry_run_errors_never_advance_schedule() {
    for dry_run in [false, true] {
        let boundary =
            Boundary::with_rows(vec![row("a", "same fact", 1), row("b", "Same fact!", 2)]);
        let before = boundary.snapshot();
        boundary.fail_once("memory::list");
        let cfg = WorkerConfig {
            semantic_enabled: false,
            ..WorkerConfig::default()
        };
        let response = run(&boundary, cfg, dry_run).await;
        assert!(!response.checkpointed);
        assert_eq!(response.superseded, 0);
        assert!(!response.banks[0].errors.is_empty());
        assert_eq!(before["rows"], boundary.snapshot()["rows"]);
        assert_eq!(
            before["state"]["last_run"],
            boundary.snapshot()["state"]["last_run"]
        );
        assert!(
            !boundary
                .calls()
                .iter()
                .any(|(f, p)| f == "state::set" && p["key"] == "last_run")
        );
        if dry_run {
            assert_eq!(before, boundary.snapshot());
        }
    }
}

#[tokio::test]
async fn zero_budget_and_two_pinned_records_never_emit_mutations() {
    for both_pinned in [false, true] {
        let mut rows = vec![row("a", "Same fact", 1), row("b", "same fact!", 2)];
        if both_pinned {
            for row in &mut rows {
                row["pinned"] = json!(true);
            }
        }
        let boundary = Boundary::with_rows(rows);
        let cfg = WorkerConfig {
            max_supersedes_per_run: 0,
            semantic_enabled: false,
            ..WorkerConfig::default()
        };
        let before = boundary.snapshot();
        let response = run(&boundary, cfg, false).await;
        assert_eq!(response.superseded, 0);
        assert_eq!(response.banks[0].capped, usize::from(!both_pinned));
        assert_eq!(before["rows"], boundary.snapshot()["rows"]);
        assert!(
            boundary
                .calls()
                .iter()
                .all(|(f, _)| f != "memory::supersede")
        );
    }
}

#[test]
fn normalization_keeps_operators_so_symbol_variants_never_group() {
    assert_eq!(consolidate::normalize("limit < 5"), "limit < 5");
    assert_eq!(consolidate::normalize("limit > 5."), "limit > 5");
    assert_eq!(
        consolidate::token_key("Bob manages Alice"),
        "alice bob manages"
    );
    let rows: Vec<MemoryRow> = vec![row("a", "limit < 5", 1), row("b", "limit > 5", 2)]
        .into_iter()
        .map(|v| serde_json::from_value(v).unwrap())
        .collect();
    assert!(consolidate::plan(&rows).is_empty());
}

fn options() -> SemanticOptions {
    SemanticOptions {
        model: String::new(),
        batch: 1,
        candidates: 1,
        max_per_run: 1,
        max_llm_calls: usize::MAX,
    }
}

#[tokio::test]
async fn semantic_transport_failure_holds_cursor_until_a_supplied_reply_can_be_consumed() {
    let boundary = Boundary::with_rows(vec![
        row("a", "User prefers tea", 1),
        row("b", "Tea is the user's preferred drink", 2),
    ]);
    let before = boundary.snapshot();
    let mut budget = 1;
    let mut failed = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &options(), &mut budget, &mut failed).await;
    assert_eq!(budget, 1);
    assert_eq!(before["state"], boundary.snapshot()["state"]);
    assert!(failed.semantic.is_empty());
    assert!(
        failed
            .notes
            .iter()
            .any(|n| n.contains("no synthetic decision supplied"))
    );
    boundary.decision(json!({"actions":[{"type":"merge","survivor":"n1","retired":"c1","reason":"Synthetic supplied equivalence; not measured semantics."}]}));
    let mut report = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &options(), &mut budget, &mut report).await;
    assert_eq!(budget, 0);
    assert_eq!(report.superseded, 1);
    assert_eq!(report.semantic.len(), 1);
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":1,"id":"a"})
    );
    assert_eq!(
        boundary
            .calls()
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .count(),
        1
    );
    assert!(
        boundary
            .calls()
            .iter()
            .all(|(f, _)| f != "router::models::list"
                && f != "router::provider::list"
                && f != "router::embed")
    );
}

#[tokio::test]
async fn operator_variants_never_reach_the_optional_reorder_tier() {
    let boundary = Boundary::with_rows(vec![row("a", "limit < 5", 1), row("b", "limit > 5", 2)]);
    let cfg = WorkerConfig {
        semantic_enabled: false,
        llm_assist_enabled: true,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    };
    let response = run(&boundary, cfg, false).await;
    assert_eq!(response.superseded, 0);
    assert!(response.banks[0].groups.is_empty());
    assert_eq!(
        boundary
            .calls()
            .iter()
            .map(|(f, _)| f.as_str())
            .collect::<Vec<_>>(),
        [
            "memory::bank::list",
            "memory::list",
            "state::get",
            "state::set",
            "state::set"
        ]
    );
}

#[tokio::test]
async fn supplied_reorder_decision_uses_the_shared_mutation_cap() {
    let boundary = Boundary::with_rows(vec![
        row("a", "concise direct answers", 1),
        row("b", "direct concise answers", 2),
    ]);
    boundary
        .0
        .lock()
        .unwrap()
        .replies
        .push_back(Ok(json!("{\"merge_groups\":[0],\"promotions\":[]}")));
    let cfg = WorkerConfig {
        max_supersedes_per_run: 1,
        semantic_enabled: false,
        llm_assist_enabled: true,
        promote_corroboration_threshold: 0,
        ..WorkerConfig::default()
    };
    let response = run(&boundary, cfg, false).await;
    assert_eq!(response.superseded, 1);
    assert_eq!(response.banks[0].llm_merged, 1);
    let bank = &response.banks[0];
    assert_eq!(bank.llm_calls, 1);
    assert_eq!((bank.llm_input_tokens, bank.llm_output_tokens), (60, 5));
    assert_eq!(bank.llm_cost_usd, Some(0.001));
    let calls = boundary.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(f, _)| f == "offline::reorder-decision")
            .count(),
        1
    );
    assert!(calls.iter().all(|(f, _)| !f.starts_with("memory::rule::")));
}

#[tokio::test]
async fn paged_scan_uses_offsets_and_one_budget_across_banks() {
    let rows: Vec<Value> = (0..501)
        .map(|i| row(&format!("id{i:03}"), "Same duplicate", i + 1))
        .collect();
    let boundary = Boundary::with_rows(rows);
    let cfg = WorkerConfig {
        banks: vec!["offline".into(), "second".into()],
        semantic_enabled: false,
        max_supersedes_per_run: 1,
        ..WorkerConfig::default()
    };
    // The mock exposes the same live snapshot for both banks; this tests
    // the handler's global cap, not cross-bank Memory storage isolation.
    let response = run(&boundary, cfg, false).await;
    assert_eq!(response.superseded, 1);
    assert_eq!(response.banks.len(), 2);
    assert_eq!(response.banks[0].capped, 499);
    assert_eq!(response.banks[1].superseded, 0);
    assert_eq!(response.banks[1].capped, 499);
    let calls = boundary.calls();
    let first_offsets: Vec<Value> = calls
        .iter()
        .filter(|(f, p)| f == "memory::list" && p["bank"] == "offline")
        .map(|(_, p)| p["offset"].clone())
        .collect();
    assert_eq!(first_offsets, vec![json!(0), json!(500)]);
    assert_eq!(
        calls
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .count(),
        1
    );
}

#[tokio::test]
async fn unreadable_or_unsaved_semantic_cursor_never_erases_progress() {
    for failed_function in [
        "state::get",
        "state::set:semantic_cursor:offline",
        "memory::supersede",
    ] {
        let boundary = Boundary::with_rows(vec![
            row("a", "User prefers tea", 1),
            row("b", "Tea is the user's preferred drink", 2),
        ]);
        boundary.decision(json!({"actions":[{"type":"merge","survivor":"n1","retired":"c1","reason":"Synthetic supplied equivalence."}]}));
        boundary.fail_once(failed_function);
        let mut budget = 1;
        let mut report = consolidate::BankReport::default();
        semantic::run_semantic_tier(&boundary, "offline", &options(), &mut budget, &mut report)
            .await;
        assert_eq!(
            boundary.snapshot()["state"]["semantic_cursor:offline"],
            json!({"updated_at":0,"id":""})
        );
        match failed_function {
            "state::get" => {
                assert_eq!(budget, 1);
                assert!(report.notes.iter().any(|n| n.contains("cursor unreadable")));
                assert!(
                    !boundary
                        .calls()
                        .iter()
                        .any(|(f, _)| f == "router::complete")
                );
            }
            "memory::supersede" => {
                assert_eq!(budget, 1);
                assert_eq!(report.superseded, 0);
                assert!(!report.errors.is_empty());
                assert!(
                    boundary.snapshot()["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|r| r["invalid_at"].is_null())
                );
            }
            _ => {
                assert_eq!(budget, 0);
                assert_eq!(report.superseded, 1);
                assert!(report.notes.iter().any(|n| n.contains("cursor not saved")));
            }
        }
    }
}

#[tokio::test]
async fn malformed_decision_bisects_and_failed_supersede_retries_with_cursor_held() {
    let boundary = Boundary::with_rows(vec![
        row("a", "User prefers tea", 1),
        row("b", "Tea is the user's preferred drink", 2),
    ]);
    boundary.decision(json!({"missing_actions":true}));
    boundary.decision(json!({"actions":[]}));
    boundary.decision(json!({"actions":[]}));
    let mut opts = options();
    opts.batch = 2;
    opts.max_per_run = 2;
    let mut budget = 1;
    let mut report = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &opts, &mut budget, &mut report).await;
    assert_eq!(budget, 1);
    assert_eq!(report.superseded, 0);
    assert_eq!(
        boundary
            .calls()
            .iter()
            .filter(|(f, _)| f == "router::complete")
            .count(),
        3
    );
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":2,"id":"b"})
    );

    let boundary = Boundary::with_rows(vec![
        row("a", "User prefers tea", 1),
        row("b", "Tea is the user's preferred drink", 2),
    ]);
    let decision = json!({"actions":[{"type":"merge","survivor":"n1","retired":"c1","reason":"Synthetic supplied equivalence."}]});
    boundary.decision(decision.clone());
    boundary.fail_once("memory::supersede");
    let mut first = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &options(), &mut budget, &mut first).await;
    assert_eq!(budget, 1);
    assert_eq!(first.superseded, 0);
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":0,"id":""})
    );
    boundary.decision(decision);
    let mut retry = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &options(), &mut budget, &mut retry).await;
    assert_eq!(budget, 0);
    assert_eq!(retry.superseded, 1);
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":1,"id":"a"})
    );
}

fn three_duplicates(guarded: bool) -> Boundary {
    let boundary = Boundary::with_rows(vec![
        row("a", "Same fact", 1),
        row("b", "same fact.", 2),
        row("c", "SAME  fact", 3),
    ]);
    boundary.0.lock().unwrap().guarded = guarded;
    boundary
}

#[tokio::test]
async fn an_older_memory_worker_still_retires_a_whole_group_in_one_run() {
    // No `survivor` in the reply: the worker predates the guards and ignores
    // `expected_*`, so the rest of the group goes without the winner guard.
    let boundary = three_duplicates(false);
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert_eq!(response.superseded, 2);
    let calls = supersedes(&boundary);
    assert_eq!(guarded_id(&calls[0]), "b");
    assert_eq!(calls[1]["id"], "c");
    assert_eq!(calls[1]["expected_memory"]["id"], "c");
    assert!(calls[1].get("expected_superseded_by").is_none());
}

#[tokio::test]
async fn each_supersede_is_guarded_by_the_survivor_memory_returned() {
    let boundary = three_duplicates(true);
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert_eq!(response.superseded, 2);
    assert_eq!(response.banks[0].reinforced, 2);
    let calls = supersedes(&boundary);
    assert_eq!(calls.iter().map(guarded_id).collect::<Vec<_>>(), ["b", "c"]);
    assert_eq!(calls[0]["expected_superseded_by"]["revision"], 0);
    assert_eq!(calls[1]["expected_superseded_by"]["revision"], 1);
    assert_eq!(calls[1]["expected_superseded_by"]["corroboration"], 1);
}

#[tokio::test]
async fn a_record_edited_after_the_scan_is_reported_and_never_retired() {
    for edited in ["a", "b"] {
        let boundary =
            Boundary::with_rows(vec![row("a", "same fact", 1), row("b", "Same fact!", 2)]);
        {
            let mut inner = boundary.0.lock().unwrap();
            inner.guarded = true;
            inner.stale = Some(edited.into());
        }
        let response = run(&boundary, deterministic_only(), false).await;
        assert!(!response.checkpointed, "{edited}");
        assert_eq!(response.superseded, 0);
        assert!(response.banks[0].errors[0].contains("changed since expected snapshot"));
        assert!(
            boundary.snapshot()["rows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["invalid_at"].is_null())
        );
    }
}

#[tokio::test]
async fn a_semantic_merge_keeps_the_provenance_snapshot_guard() {
    let boundary = Boundary::with_rows(vec![
        row("old", "User prefers tea", 1),
        row("new", "Tea is the user's preferred drink", 2),
    ]);
    {
        let mut inner = boundary.0.lock().unwrap();
        inner.guarded = true;
        inner.listed = Some(HashSet::from(["new".to_string()]));
        // A provenance-only edit: the text the judge read is unchanged.
        inner.stale = Some("new".into());
    }
    boundary.decision(json!({"actions":[{"type":"merge","survivor":"c1","retired":"n1","reason":"Synthetic supplied equivalence."}]}));
    let response = run(&boundary, semantic_only(), false).await;
    assert!(!response.checkpointed);
    assert_eq!(response.superseded, 0);
    assert!(response.banks[0].errors[0].contains("changed since expected snapshot"));
    assert_eq!(supersedes(&boundary).len(), 1);
    assert!(
        boundary.snapshot()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["invalid_at"].is_null())
    );
}

#[tokio::test]
async fn two_merges_into_one_survivor_in_a_batch_expect_its_returned_snapshot() {
    let boundary = Boundary::with_rows(vec![
        row("old", "User drinks tea every morning.", 1),
        row("new1", "Every morning the user drinks tea.", 2),
        row("new2", "Each morning the user has tea.", 3),
    ]);
    {
        let mut inner = boundary.0.lock().unwrap();
        inner.guarded = true;
        inner.listed = Some(HashSet::from(["new1".to_string(), "new2".to_string()]));
    }
    boundary.decision(json!({"actions":[
        {"type":"merge","survivor":"c1","retired":"n1","reason":"Synthetic supplied equivalence."},
        {"type":"merge","survivor":"c1","retired":"n2","reason":"Synthetic supplied equivalence."}
    ]}));
    let cfg = WorkerConfig {
        semantic_batch: 2,
        semantic_max_per_run: 2,
        ..semantic_only()
    };
    let response = run(&boundary, cfg, false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert_eq!(response.superseded, 2);
    let calls = supersedes(&boundary);
    assert_eq!(
        calls.iter().map(guarded_id).collect::<Vec<_>>(),
        ["new1", "new2"]
    );
    assert_eq!(calls[1]["expected_superseded_by"]["revision"], 1);
}

#[tokio::test]
async fn a_survivor_reinforced_in_an_earlier_batch_is_guarded_by_its_returned_snapshot() {
    // All three are pending, one per batch. Batch 1 merges `a` into `b`, a
    // candidate that is itself pending in batch 2; batch 2 then acts on `b`.
    let boundary = Boundary::with_rows(vec![
        row("a", "User drinks tea every morning.", 1),
        row("b", "Every morning the user drinks green tea.", 2),
        row("c", "Each morning the user has green tea.", 3),
    ]);
    boundary.0.lock().unwrap().guarded = true;
    boundary.decision(json!({"actions":[{"type":"merge","survivor":"c1","retired":"n1","reason":"Synthetic supplied equivalence."}]}));
    boundary.decision(json!({"actions":[{"type":"merge","survivor":"n1","retired":"c1","reason":"Synthetic supplied equivalence."}]}));
    let opts = SemanticOptions {
        max_per_run: 3,
        ..options()
    };
    let mut budget = 3;
    let mut report = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &opts, &mut budget, &mut report).await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.superseded, 2);
    let calls = supersedes(&boundary);
    assert_eq!(calls.iter().map(guarded_id).collect::<Vec<_>>(), ["a", "c"]);
    assert_eq!(calls[1]["superseded_by"], "b");
    assert_eq!(calls[1]["expected_superseded_by"]["revision"], 1);
}

/// Every `dup_of` pair of memory's recall golden set, supplied as the judge's
/// merge decision, passes the validator and reaches `memory::supersede`.
#[tokio::test]
async fn golden_duplicate_pairs_pass_the_validator_as_supplied_merges() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../memory/tests/recall_quality/golden.json");
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let memories = golden["memories"].as_array().unwrap();
    let mut pairs = 0;
    for dup in memories.iter().filter(|m| m["dup_of"].is_string()) {
        let original = memories.iter().find(|m| m["key"] == dup["dup_of"]).unwrap();
        let key = dup["key"].as_str().unwrap();
        let boundary = Boundary::with_rows(vec![
            row(key, dup["text"].as_str().unwrap(), 2),
            row(
                original["key"].as_str().unwrap(),
                original["text"].as_str().unwrap(),
                1,
            ),
        ]);
        {
            let mut inner = boundary.0.lock().unwrap();
            inner.guarded = true;
            inner.listed = Some(HashSet::from([key.to_string()]));
        }
        boundary.decision(json!({"actions":[{"type":"merge","survivor":"c1","retired":"n1","reason":"Golden duplicate pair."}]}));
        let response = run(&boundary, semantic_only(), false).await;
        assert_eq!(response.banks[0].semantic_rejected, 0, "{key}");
        assert!(
            response.checkpointed,
            "{key}: {:?}",
            response.banks[0].errors
        );
        assert_eq!(
            supersedes(&boundary)
                .iter()
                .map(guarded_id)
                .collect::<Vec<_>>(),
            [key]
        );
        pairs += 1;
    }
    assert!(pairs > 0, "the golden set has no dup_of pairs");
}

fn model_calls(boundary: &Boundary) -> usize {
    boundary
        .calls()
        .iter()
        .filter(|(f, _)| f == "router::complete")
        .count()
}

#[tokio::test]
async fn a_batch_where_no_reply_parses_holds_the_cursor() {
    let boundary = Boundary::with_rows(vec![
        row("a", "User prefers tea", 1),
        row("b", "Tea is the user's preferred drink", 2),
    ]);
    for _ in 0..3 {
        boundary.decision(json!({"missing_actions":true}));
    }
    let mut opts = options();
    opts.batch = 2;
    opts.max_per_run = 2;
    let mut budget = 1;
    let mut report = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &opts, &mut budget, &mut report).await;
    // The pair, then each record alone: the judge itself is failing.
    assert_eq!(model_calls(&boundary), 3);
    assert_eq!(report.llm_calls, 3);
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":0,"id":""})
    );
    assert!(report.semantic_failed.is_empty());
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("cursor held before `a`")),
        "{:?}",
        report.notes
    );
}

#[tokio::test]
async fn a_poison_record_is_skipped_once_a_sibling_parses() {
    let boundary = Boundary::with_rows(vec![
        row("a", "User prefers tea", 1),
        row("b", "Tea is the user's preferred drink", 2),
    ]);
    boundary.decision(json!({"missing_actions":true}));
    boundary.decision(json!({"missing_actions":true}));
    boundary.decision(json!({"actions":[]}));
    let mut opts = options();
    opts.batch = 2;
    opts.max_per_run = 2;
    let mut budget = 1;
    let mut report = consolidate::BankReport::default();
    semantic::run_semantic_tier(&boundary, "offline", &opts, &mut budget, &mut report).await;
    assert_eq!(model_calls(&boundary), 3);
    assert_eq!(report.semantic_failed, ["a"]);
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":2,"id":"b"})
    );
}

#[tokio::test]
async fn a_lone_record_is_poison_once_a_later_batch_parses() {
    let rows = || {
        vec![
            row("a", "User prefers tea", 1),
            row("b", "Tea is the user's preferred drink", 2),
            row("c", "Tea is what the user drinks", 3),
        ]
    };
    let mut opts = options();
    opts.max_per_run = 3;
    let held = json!({"updated_at":0,"id":""});

    // One record per batch: the first reply fails, the next one parses.
    let boundary = Boundary::with_rows(rows());
    boundary.decision(json!({"missing_actions":true}));
    boundary.decision(json!({"actions":[]}));
    boundary.decision(json!({"actions":[]}));
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(&boundary, "offline", &opts, &mut budget, &mut report).await;
    assert_eq!(report.semantic_failed, ["a"]);
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        json!({"updated_at":3,"id":"c"})
    );

    // Two batches with failed replies and none parsed: the judge is failing.
    let boundary = Boundary::with_rows(rows());
    for _ in 0..3 {
        boundary.decision(json!({"missing_actions":true}));
    }
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(&boundary, "offline", &opts, &mut budget, &mut report).await;
    assert_eq!(model_calls(&boundary), 2, "stopped before `c`");
    assert!(report.semantic_failed.is_empty());
    assert_eq!(
        boundary.snapshot()["state"]["semantic_cursor:offline"],
        held
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("cursor held before `a`")),
        "{:?}",
        report.notes
    );
}

fn ids(list: &[&str]) -> Option<HashSet<String>> {
    Some(list.iter().map(|id| id.to_string()).collect())
}

fn stored_cursor(boundary: &Boundary) -> Value {
    boundary.snapshot()["state"]["semantic_cursor:offline"].clone()
}

fn wait_since(boundary: &Boundary, ms: u64) {
    boundary.0.lock().unwrap().state.insert(
        "semantic_cursor:offline".into(),
        json!({"updated_at":0,"id":"","waiting_since":ms}),
    );
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn notes_with(report: &consolidate::BankReport, text: &str) -> usize {
    report.notes.iter().filter(|n| n.contains(text)).count()
}

fn tea_rows(n: usize) -> Vec<Value> {
    [
        "User prefers tea",
        "Tea is the user's preferred drink",
        "Tea is what the user drinks",
        "User drinks coffee at night",
    ]
    .iter()
    .zip(["a", "b", "c", "d"])
    .zip(1..)
    .take(n)
    .map(|((text, id), at)| row(id, text, at))
    .collect()
}

fn pair_options(max_per_run: usize) -> SemanticOptions {
    SemanticOptions {
        batch: 2,
        max_per_run,
        ..options()
    }
}

#[tokio::test]
async fn a_pass_during_a_re_embed_window_finds_the_pair_once_vectors_land() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().vectors = ids(&[]);
    let cfg = || WorkerConfig {
        semantic_batch: 2,
        semantic_max_per_run: 2,
        ..semantic_only()
    };
    let first = run(&boundary, cfg(), false).await;
    let bank = &first.banks[0];
    assert_eq!(model_calls(&boundary), 0);
    assert_eq!(bank.llm_calls, 0);
    assert!(bank.errors.is_empty(), "{:?}", bank.errors);
    assert_eq!(notes_with(bank, "semantic tier deferred: 2 of 2"), 1);
    assert!(first.checkpointed);
    let held = stored_cursor(&boundary);
    assert_eq!((&held["updated_at"], &held["id"]), (&json!(0), &json!("")));
    assert!(held["waiting_since"].as_u64().is_some(), "{held}");

    boundary.0.lock().unwrap().vectors = ids(&["a", "b"]);
    boundary.decision(json!({"actions":[{"type":"merge","survivor":"n1","retired":"n2","reason":"Synthetic supplied equivalence."}]}));
    let second = run(&boundary, cfg(), false).await;
    assert_eq!(second.superseded, 1);
    assert_eq!(stored_cursor(&boundary), json!({"updated_at":2,"id":"b"}));
}

#[tokio::test]
async fn a_thin_semantic_arm_defers_before_any_judge_call() {
    let boundary = Boundary::with_rows(tea_rows(3));
    boundary.0.lock().unwrap().vectors = ids(&["c"]);
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(3),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 0);
    let held = stored_cursor(&boundary);
    assert_eq!((&held["updated_at"], &held["id"]), (&json!(0), &json!("")));
    assert_eq!(
        notes_with(&report, "deferred: 2 of 3"),
        1,
        "{:?}",
        report.notes
    );
}

#[tokio::test]
async fn vectors_that_never_land_are_judged_after_the_wait_then_waited_for_again() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().vectors = ids(&["b"]);
    wait_since(&boundary, 1);
    boundary.decision(json!({"actions":[]}));
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 1);
    assert_eq!(
        notes_with(&report, "judged without 1 of 2"),
        1,
        "{:?}",
        report.notes
    );
    assert_eq!(stored_cursor(&boundary), json!({"updated_at":2,"id":"b"}));

    boundary
        .0
        .lock()
        .unwrap()
        .rows
        .push(row("c", "Tea is what the user drinks", 3));
    let mut report = consolidate::BankReport::default();
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 1);
    assert_eq!(
        notes_with(&report, "deferred: 2 of 3"),
        1,
        "{:?}",
        report.notes
    );
    let held = stored_cursor(&boundary);
    assert_eq!((&held["updated_at"], &held["id"]), (&json!(2), &json!("b")));
    assert!(held["waiting_since"].as_u64().unwrap() > 1, "{held}");
}

#[tokio::test]
async fn a_run_soon_after_a_deferral_keeps_waiting() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().vectors = ids(&["b"]);
    let since = now_ms();
    wait_since(&boundary, since);
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 0);
    assert_eq!(
        notes_with(&report, "deferred: 1 of 2"),
        1,
        "{:?}",
        report.notes
    );
    assert_eq!(
        stored_cursor(&boundary),
        json!({"updated_at":0,"id":"","waiting_since":since})
    );
    // An unchanged wait is not written again.
    assert!(
        boundary
            .calls()
            .iter()
            .all(|(f, p)| !(f == "state::set" && p["key"] == "semantic_cursor:offline"))
    );
}

#[tokio::test]
async fn a_bank_with_no_vectors_is_never_judged_thin() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().vectors = ids(&[]);
    wait_since(&boundary, 1);
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 0);
    assert_eq!(
        notes_with(&report, "deferred: 2 of 2"),
        1,
        "{:?}",
        report.notes
    );
    assert_eq!(
        stored_cursor(&boundary),
        json!({"updated_at":0,"id":"","waiting_since":1})
    );
}

#[tokio::test]
async fn a_deferral_mid_pass_keeps_the_judged_batches() {
    let boundary = Boundary::with_rows(tea_rows(4));
    boundary.0.lock().unwrap().judge_vectors = ids(&["a", "b", "c"]);
    boundary.decision(json!({"actions":[]}));
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(4),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 1);
    assert_eq!(
        notes_with(&report, "deferred: 1 of 4"),
        1,
        "{:?}",
        report.notes
    );
    let held = stored_cursor(&boundary);
    assert_eq!((&held["updated_at"], &held["id"]), (&json!(2), &json!("b")));
    assert!(held["waiting_since"].as_u64().is_some(), "{held}");
}

#[tokio::test]
async fn an_older_memory_worker_without_coverage_is_judged_as_before_with_a_note() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().no_coverage = true;
    boundary.decision(json!({"actions":[{"type":"merge","survivor":"n1","retired":"n2","reason":"Synthetic supplied equivalence."}]}));
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(report.superseded, 1);
    assert_eq!(stored_cursor(&boundary), json!({"updated_at":2,"id":"b"}));
    assert_eq!(report.notes, [semantic::NOTE_NO_COVERAGE]);
}

#[tokio::test]
async fn a_failed_save_of_the_wait_is_one_note() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().vectors = ids(&[]);
    boundary.fail_once("state::set:semantic_cursor:offline");
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 0);
    assert_eq!(notes_with(&report, "semantic tier deferred: 2 of 2"), 1);
    assert_eq!(
        notes_with(&report, "semantic cursor not saved"),
        1,
        "{:?}",
        report.notes
    );
    // The failed write is the wait, not a cursor move.
    let cursor_writes: Vec<Value> = boundary
        .calls()
        .into_iter()
        .filter(|(f, p)| f == "state::set" && p["key"] == "semantic_cursor:offline")
        .map(|(_, p)| p["value"].clone())
        .collect();
    assert_eq!(cursor_writes.len(), 1, "{cursor_writes:?}");
    assert!(
        cursor_writes[0]["waiting_since"].as_u64().is_some(),
        "{cursor_writes:?}"
    );
    assert_eq!(stored_cursor(&boundary), json!({"updated_at":0,"id":""}));
}

#[tokio::test]
async fn a_failed_cursor_save_after_judging_stops_the_tier_with_one_note() {
    let boundary = Boundary::with_rows(tea_rows(2));
    boundary.0.lock().unwrap().vectors = ids(&["b"]);
    wait_since(&boundary, 1);
    boundary.decision(json!({"actions":[]}));
    boundary.fail_once("state::set:semantic_cursor:offline");
    let (mut budget, mut report) = (1, consolidate::BankReport::default());
    semantic::run_semantic_tier(
        &boundary,
        "offline",
        &pair_options(2),
        &mut budget,
        &mut report,
    )
    .await;
    assert_eq!(model_calls(&boundary), 1);
    assert_eq!(
        notes_with(&report, "semantic cursor not saved"),
        1,
        "{:?}",
        report.notes
    );
    // No second save after the loop: the stored wait and cursor stay as they were.
    assert_eq!(
        stored_cursor(&boundary),
        json!({"updated_at":0,"id":"","waiting_since":1})
    );
}

#[tokio::test]
async fn an_unreadable_report_ring_is_never_overwritten() {
    let boundary = Boundary::with_rows(vec![row("a", "User prefers tea", 1)]);
    boundary.fail_once("state::get:reports");
    let cfg = WorkerConfig {
        semantic_enabled: false,
        ..WorkerConfig::default()
    };
    run(&boundary, cfg, false).await;
    assert_eq!(
        boundary.snapshot()["state"]["reports"],
        json!([{"previous":true}])
    );
    assert!(
        boundary
            .calls()
            .iter()
            .all(|(f, p)| !(f == "state::set" && p["key"] == "reports"))
    );
}

#[tokio::test]
async fn the_llm_call_cap_defers_the_rest_across_banks() {
    let boundary = Boundary::with_rows(vec![
        row("a", "User prefers tea", 1),
        row("b", "Tea is the user's preferred drink", 2),
    ]);
    boundary.0.lock().unwrap().replies.push_back(Ok(json!({
        "message":{"stop_reason":"stop","content":[{"type":"text","text":"{\"actions\":[]}"}]},
        "usage":{"input":100,"cache_read":20,"output":7,"cost_usd":0.002}
    })));
    let cfg = WorkerConfig {
        banks: vec!["offline".into(), "second".into()],
        max_llm_calls_per_run: 1,
        ..semantic_only()
    };
    let response = run(&boundary, cfg, false).await;
    assert_eq!(model_calls(&boundary), 1);
    let (first, second) = (&response.banks[0], &response.banks[1]);
    assert_eq!(first.llm_calls, 1);
    assert_eq!(first.llm_input_tokens, 120);
    assert_eq!(first.llm_output_tokens, 7);
    assert_eq!(first.llm_cost_usd, Some(0.002));
    assert_eq!(second.llm_calls, 0);
    assert!(
        second
            .notes
            .iter()
            .any(|n| n.contains("per-run LLM call cap reached")),
        "{:?}",
        second.notes
    );
    assert!(
        boundary.snapshot()["state"]
            .get("semantic_cursor:second")
            .is_none()
    );
}

#[tokio::test]
async fn a_zero_call_cap_defers_both_tiers() {
    let boundary = Boundary::with_rows(vec![
        row("a", "concise direct answers", 1),
        row("b", "direct concise answers", 2),
    ]);
    let cfg = WorkerConfig {
        llm_assist_enabled: true,
        semantic_enabled: true,
        max_llm_calls_per_run: 0,
        ..WorkerConfig::default()
    };
    let response = run(&boundary, cfg, false).await;
    assert!(
        boundary
            .calls()
            .iter()
            .all(|(f, _)| f != "router::complete" && f != "offline::reorder-decision")
    );
    let notes = &response.banks[0].notes;
    assert!(
        notes.iter().any(|n| n.contains("llm tier deferred")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("semantic tier deferred")),
        "{notes:?}"
    );
}

fn corroborated(id: &str, text: &str, at: u64) -> Value {
    let mut r = row(id, text, at);
    r["corroboration"] = json!(5);
    r
}

fn promotion_only() -> WorkerConfig {
    WorkerConfig {
        semantic_enabled: false,
        llm_assist_enabled: true,
        promote_corroboration_threshold: 4,
        ..WorkerConfig::default()
    }
}

fn payloads(boundary: &Boundary, function: &str) -> Vec<Value> {
    boundary
        .calls()
        .into_iter()
        .filter(|(f, _)| f == function)
        .map(|(_, p)| p)
        .collect()
}

/// One promotable memory; `learned` and the guard as the test sets them.
fn one_candidate(learned: &str, guarded: bool) -> Boundary {
    let boundary = Boundary::with_rows(vec![corroborated(
        "fp-brief",
        "User wants brief answers",
        1,
    )]);
    {
        let mut inner = boundary.0.lock().unwrap();
        inner.learned = learned.into();
        inner.guarded = guarded;
    }
    boundary.judge_reply(json!({"promotions":[{"id":"fp-brief","rule":"Keep answers brief."}]}));
    boundary
}

#[tokio::test]
async fn a_second_promotion_pass_over_the_same_candidates_adds_nothing() {
    let boundary = Boundary::with_rows(vec![
        corroborated("fp-brief", "User wants brief answers", 1),
        corroborated("fp-cite", "User wants sources cited", 2),
    ]);
    boundary.0.lock().unwrap().guarded = true;
    boundary.judge_reply(json!({"promotions":[
        {"id":"fp-brief","rule":"Keep answers brief."},
        {"id":"fp-cite","rule":"Cite sources."}
    ]}));
    let first = run(&boundary, promotion_only(), false).await;
    assert_eq!(first.banks[0].promoted, 2);
    assert!(first.banks[0].errors.is_empty(), "{:?}", first.banks[0]);
    assert_eq!(
        boundary.snapshot()["state"]["promoted:offline"],
        json!(["fp-brief", "fp-cite"])
    );
    let learned = boundary.learned();

    // No reply queued: a judge call would surface as a skip note.
    let second = run(&boundary, promotion_only(), false).await;
    let bank = &second.banks[0];
    assert_eq!(bank.promoted, 0);
    assert!(bank.errors.is_empty() && bank.notes.is_empty(), "{bank:?}");
    assert_eq!(payloads(&boundary, "offline::reorder-decision").len(), 1);
    assert_eq!(payloads(&boundary, "memory::rule::set").len(), 1);
    assert_eq!(boundary.learned(), learned);
}

#[tokio::test]
async fn a_promotion_merges_again_when_the_rule_changes_under_it() {
    let boundary = one_candidate("# Learned\n- Existing line.\n", true);
    boundary.0.lock().unwrap().rule_edit = Some("- Typed by hand meanwhile.\n".into());
    let response = run(&boundary, promotion_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(bank.promoted, 1);
    assert!(bank.errors.is_empty() && bank.notes.is_empty(), "{bank:?}");
    let sets = payloads(&boundary, "memory::rule::set");
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0]["expected"], "# Learned\n- Existing line.\n");
    assert_eq!(
        sets[1]["expected"],
        "# Learned\n- Existing line.\n- Typed by hand meanwhile.\n"
    );
    assert_eq!(
        boundary.learned(),
        "# Learned\n- Existing line.\n- Typed by hand meanwhile.\n- Keep answers brief.\n"
    );
    assert_eq!(
        boundary.snapshot()["state"]["promoted:offline"],
        json!(["fp-brief"])
    );
}

#[tokio::test]
async fn a_covered_candidate_is_recorded_and_not_judged_again() {
    let boundary = Boundary::with_rows(vec![corroborated(
        "fp-brief",
        "User wants brief answers",
        1,
    )]);
    boundary.0.lock().unwrap().learned = "# Learned\n- Keep answers brief.\n".into();
    assert!(
        boundary.snapshot()["state"]
            .get("promoted:offline")
            .is_none(),
        "promoted before the ledger existed"
    );
    boundary.judge_reply(json!({"covered":["fp-brief"]}));
    let response = run(&boundary, promotion_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(bank.promoted, 0);
    assert!(bank.errors.is_empty(), "{bank:?}");
    assert!(payloads(&boundary, "memory::rule::set").is_empty());
    assert_eq!(
        boundary.snapshot()["state"]["promoted:offline"],
        json!(["fp-brief"])
    );
    let judged = payloads(&boundary, "offline::reorder-decision");
    let prompt: Value = serde_json::from_str(judged[0]["prompt"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["learned_lines"], json!(["Keep answers brief."]));

    run(&boundary, promotion_only(), false).await;
    assert_eq!(payloads(&boundary, "offline::reorder-decision").len(), 1);
}

#[tokio::test]
async fn a_covered_verdict_without_learned_lines_is_not_recorded() {
    let boundary = Boundary::with_rows(vec![corroborated(
        "fp-brief",
        "User wants brief answers",
        1,
    )]);
    boundary.judge_reply(json!({"covered":["fp-brief"]}));
    boundary.judge_reply(json!({"covered":["fp-brief"]}));
    for _ in 0..2 {
        let response = run(&boundary, promotion_only(), false).await;
        let bank = &response.banks[0];
        assert!(bank.errors.is_empty() && bank.notes.is_empty(), "{bank:?}");
        assert!(
            boundary.snapshot()["state"]
                .get("promoted:offline")
                .is_none()
        );
    }
    assert!(payloads(&boundary, "memory::rule::set").is_empty());
    // Not in the ledger, so the candidate is judged again.
    assert_eq!(payloads(&boundary, "offline::reorder-decision").len(), 2);
}

#[tokio::test]
async fn the_rule_is_read_again_after_the_judge_call() {
    // Unguarded, so a write built from the pre-judge read would drop the edit.
    let boundary = one_candidate("# Learned\n- Existing line.\n", false);
    boundary.0.lock().unwrap().judge_edit = Some("- Typed during the judge call.\n".into());
    run(&boundary, promotion_only(), false).await;
    let order: Vec<String> = boundary
        .calls()
        .into_iter()
        .map(|(f, _)| f)
        .filter(|f| f.starts_with("memory::rule::") || f == "offline::reorder-decision")
        .collect();
    assert_eq!(
        order,
        [
            "memory::rule::list",
            "offline::reorder-decision",
            "memory::rule::list",
            "memory::rule::set"
        ]
    );
    let after_judge = "# Learned\n- Existing line.\n- Typed during the judge call.\n";
    assert_eq!(
        payloads(&boundary, "memory::rule::set")[0]["expected"],
        after_judge
    );
    assert_eq!(
        boundary.learned(),
        format!("{after_judge}- Keep answers brief.\n")
    );
}

/// A note an agent saved from inside a turn.
fn agent_note(id: &str, text: &str, at: u64) -> Value {
    let mut r = row(id, text, at);
    r["confidence"] = json!("stated");
    r["source"]["turn_id"] = json!("t-1");
    r
}

/// A line the learned rule's cap moved back to memories.
fn demoted(id: &str, text: &str, at: u64) -> Value {
    let mut r = row(id, text, at);
    r["tags"] = json!(["demoted-rule"]);
    r
}

#[tokio::test]
async fn an_agents_note_and_a_demoted_line_are_never_offered_for_promotion() {
    let mut guess = agent_note("fp-guess", "User indents with tabs", 1);
    guess["corroboration"] = json!(5);
    let mut pushed_out = demoted("fp-demoted", "User wants emojis", 2);
    pushed_out["corroboration"] = json!(5);
    let boundary = Boundary::with_rows(vec![
        guess.clone(),
        pushed_out.clone(),
        corroborated("fp-brief", "User wants brief answers", 3),
    ]);
    boundary.judge_reply(json!({"promotions":[
        {"id":"fp-guess","rule":"Indent with tabs."},
        {"id":"fp-demoted","rule":"Use emojis."},
        {"id":"fp-brief","rule":"Keep answers brief."}
    ]}));
    let response = run(&boundary, promotion_only(), false).await;
    assert_eq!(response.banks[0].promoted, 1, "{:?}", response.banks[0]);
    let judged = payloads(&boundary, "offline::reorder-decision");
    let prompt = judged[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("fp-brief"), "{prompt}");
    assert!(!prompt.contains("fp-guess") && !prompt.contains("fp-demoted"));
    assert!(boundary.learned().ends_with("\n- Keep answers brief.\n"));
    assert!(!boundary.learned().contains("tabs") && !boundary.learned().contains("emojis"));

    let ineligible = Boundary::with_rows(vec![guess, pushed_out]);
    let response = run(&ineligible, promotion_only(), false).await;
    let bank = &response.banks[0];
    assert!(bank.errors.is_empty() && bank.notes.is_empty(), "{bank:?}");
    assert!(payloads(&ineligible, "offline::reorder-decision").is_empty());
}

#[tokio::test]
async fn a_demoted_line_merged_into_its_twin_adds_nothing_to_it() {
    let mut twin = row("twin", "same fact.", 2);
    twin["corroboration"] = json!(3);
    let exact = Boundary::with_rows(vec![demoted("line", "Same fact", 1), twin]);
    let exact_run = run(&exact, deterministic_only(), false).await;
    let semantic = Boundary::with_rows(vec![
        row("old", "User prefers tea", 1),
        demoted("new", "Tea is the user's preferred drink", 2),
    ]);
    semantic.0.lock().unwrap().listed = Some(HashSet::from(["new".to_string()]));
    semantic.decision(json!({"actions":[{"type":"merge","survivor":"c1","retired":"n1","reason":"Synthetic supplied equivalence."}]}));
    let semantic_run = run(&semantic, semantic_only(), false).await;
    for (boundary, response, retired) in [
        (&exact, exact_run, "line"),
        (&semantic, semantic_run, "new"),
    ] {
        assert!(response.checkpointed, "{:?}", response.banks[0].errors);
        let calls = supersedes(boundary);
        assert_eq!(calls.len(), 1, "{retired}");
        assert_eq!(calls[0]["id"], retired);
        assert_eq!(calls[0]["reinforce"], false, "{retired}");
        assert_eq!(response.banks[0].reinforced, 0, "{retired}");
    }
}

/// A line the user stated: confidence `stated`, no turn behind it.
fn user_stated(id: &str, text: &str, at: u64) -> Value {
    let mut r = row(id, text, at);
    r["confidence"] = json!("stated");
    r
}

/// The user's line and an agent's note with the same meaning, the note new
/// since the semantic cursor, and one supplied judge decision.
fn users_line_and_agents_note(decision: Value) -> Boundary {
    let boundary = Boundary::with_rows(vec![
        user_stated("user", "User prefers tea", 1),
        agent_note("note", "Tea is the user's preferred drink", 2),
    ]);
    {
        let mut inner = boundary.0.lock().unwrap();
        inner.guarded = true;
        inner.listed = Some(HashSet::from(["note".to_string()]));
    }
    boundary.decision(json!({"actions":[decision]}));
    boundary
}

#[tokio::test]
async fn an_exact_merge_keeps_the_users_memory_over_an_agents_note() {
    // Corroboration and age both favoured the note before.
    let mut note = agent_note("note", "Same fact", 1);
    note["corroboration"] = json!(3);
    let boundary = Boundary::with_rows(vec![note, user_stated("user", "same fact.", 2)]);
    boundary.0.lock().unwrap().guarded = true;
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    let calls = supersedes(&boundary);
    assert_eq!(calls.len(), 1);
    assert_eq!(guarded_id(&calls[0]), "note");
    assert_eq!(calls[0]["superseded_by"], "user");
    assert_eq!(calls[0]["reinforce"], false);
    assert_eq!(response.banks[0].reinforced, 0);
}

#[tokio::test]
async fn a_semantic_merge_retires_an_agents_note_into_the_users_memory() {
    // Whichever side the judge keeps: a merge kept by the note is swapped.
    for (survivor, retired) in [("c1", "n1"), ("n1", "c1")] {
        let boundary = users_line_and_agents_note(
            json!({"type":"merge","survivor":survivor,"retired":retired,"reason":"Synthetic supplied equivalence."}),
        );
        let response = run(&boundary, semantic_only(), false).await;
        assert!(response.checkpointed, "{:?}", response.banks[0].errors);
        let calls = supersedes(&boundary);
        assert_eq!(calls.len(), 1, "{survivor}");
        assert_eq!(guarded_id(&calls[0]), "note");
        assert_eq!(calls[0]["superseded_by"], "user");
        assert_eq!(calls[0]["reinforce"], false);
        assert_eq!(response.banks[0].reinforced, 0);
    }
}

#[tokio::test]
async fn an_agents_note_never_supersedes_the_users_memory() {
    let boundary = users_line_and_agents_note(
        json!({"type":"supersede","old":"c1","new":"n1","reason":"Synthetic supplied update."}),
    );
    let response = run(&boundary, semantic_only(), false).await;
    assert!(response.checkpointed);
    assert!(supersedes(&boundary).is_empty());
    assert_eq!(response.banks[0].semantic_rejected, 1);
    assert!(
        boundary.snapshot()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["invalid_at"].is_null())
    );
}

#[tokio::test]
async fn an_older_memory_worker_still_takes_the_promotion() {
    let boundary = one_candidate("", false);
    let response = run(&boundary, promotion_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(bank.promoted, 1);
    assert!(bank.errors.is_empty() && bank.notes.is_empty(), "{bank:?}");
    assert_eq!(payloads(&boundary, "memory::rule::set").len(), 1);
    let learned = boundary.learned();
    assert!(learned.starts_with("# Learned\n"), "{learned}");
    assert!(learned.ends_with("\n- Keep answers brief.\n"), "{learned}");
    assert_eq!(
        boundary.snapshot()["state"]["promoted:offline"],
        json!(["fp-brief"])
    );
}

// ---- verify tier ------------------------------------------------------------

/// A note with a recorded span; memory reads its transcript by `session`.
fn spanned(id: &str, text: &str, at: u64, confidence: &str, session: &str) -> Value {
    let mut r = row(id, text, at);
    r["confidence"] = json!(confidence);
    r["source"] = json!({"session_id":session,"first_entry_id":"e1","entry_id":"e9"});
    r
}

fn tea(confidence: &str) -> Value {
    spanned(
        "tea",
        "User drinks green tea every morning.",
        1,
        confidence,
        "s-tea",
    )
}

fn standup() -> Value {
    spanned(
        "standup",
        "User's team standup is at 9am.",
        2,
        "inferred",
        "s-standup",
    )
}

const STANDUP_MOVED: &str = "Heads up: our team standup moved to 10am, starting Monday.";

/// The transcript `memory::transcript` returns for `session`.
fn say(boundary: &Boundary, session: &str, lines: &[(&str, &str)]) {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(role, text)| json!({"role":role,"date":"2026-10-15 Thursday","text":text}))
        .collect();
    boundary
        .0
        .lock()
        .unwrap()
        .transcripts
        .insert(session.into(), json!(lines));
}

fn with_transcripts(rows: Vec<Value>) -> Boundary {
    let boundary = Boundary::with_rows(rows);
    say(
        &boundary,
        "s-tea",
        &[("user", "I drink green tea every morning.")],
    );
    say(
        &boundary,
        "s-standup",
        &[("user", STANDUP_MOVED), ("assistant", "Noted.")],
    );
    boundary
}

fn verify_only() -> WorkerConfig {
    WorkerConfig {
        verify_enabled: true,
        ..deterministic_only()
    }
}

fn verify_cursor(boundary: &Boundary) -> Value {
    boundary.snapshot()["state"]["verify_cursor:offline"].clone()
}

#[tokio::test]
async fn verify_corrects_a_contradicted_note_once_and_leaves_a_supported_one() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    boundary.decision(json!({"verdict":"supported"}));
    boundary.decision(
        json!({"verdict":"contradicted","quote":"our team standup moved to 10am",
        "corrected":"User's team standup is at 10am."}),
    );
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert!(bank.errors.is_empty(), "{bank:?}");
    assert!(response.checkpointed);
    assert_eq!(
        (bank.verified, bank.corrected, bank.verify_rejected),
        (1, 1, 0)
    );
    assert_eq!(
        payloads(&boundary, "memory::transcript")[0],
        json!({"bank":"offline","id":"tea"})
    );
    let corrections = payloads(&boundary, "memory::correct");
    assert_eq!(corrections.len(), 1);
    let fix = &corrections[0];
    assert_eq!(fix["id"], "standup");
    assert_eq!(fix["text"], "User's team standup is at 10am.");
    assert_eq!(
        fix["reason"],
        "the source conversation says: \"our team standup moved to 10am\""
    );
    assert_eq!(fix["expected_memory"]["id"], "standup");
    assert_eq!(fix["expected_memory"]["revision"], 0);
    assert_eq!(
        verify_cursor(&boundary)["first"],
        json!({"updated_at":2,"id":"standup"})
    );

    // Next pass only the correction is new, and its transcript supports it.
    boundary.decision(json!({"verdict":"supported"}));
    let again = run(&boundary, verify_only(), false).await;
    assert_eq!((again.banks[0].verified, again.banks[0].corrected), (1, 0));
    assert_eq!(payloads(&boundary, "memory::correct").len(), 1);
    assert_eq!(model_calls(&boundary), 3);
    assert_eq!(
        verify_cursor(&boundary)["first"],
        json!({"updated_at":1002,"id":"standup-corrected"})
    );
}

#[tokio::test]
async fn verify_refuses_a_correction_with_a_number_missing_from_the_transcript() {
    let boundary = with_transcripts(vec![standup()]);
    boundary.decision(
        json!({"verdict":"contradicted","quote":"our team standup moved to 10am",
        "corrected":"User's team standup is at 10:30am."}),
    );
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert!(payloads(&boundary, "memory::correct").is_empty());
    assert_eq!((bank.verify_rejected, bank.corrected), (1, 0));
    assert!(bank.errors.is_empty(), "{bank:?}");
    assert_eq!(
        verify_cursor(&boundary)["first"],
        json!({"updated_at":2,"id":"standup"})
    );
}

#[tokio::test]
async fn verify_is_off_by_default() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    let cfg = deterministic_only();
    assert!(!cfg.verify_enabled);
    let response = run(&boundary, cfg, false).await;
    assert!(payloads(&boundary, "memory::transcript").is_empty());
    assert!(payloads(&boundary, "memory::correct").is_empty());
    assert_eq!(model_calls(&boundary), 0);
    assert!(verify_cursor(&boundary).is_null());
    let bank = serde_json::to_value(&response.banks[0]).unwrap();
    for counter in ["verified", "corrected", "verify_skipped", "verify_rejected"] {
        assert!(bank.get(counter).is_none(), "{bank}");
    }
}

#[tokio::test]
async fn verify_skips_and_counts_a_missing_session() {
    let boundary = Boundary::with_rows(vec![spanned(
        "tea",
        "User drinks green tea every morning.",
        1,
        "inferred",
        "deleted-session",
    )]);
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(model_calls(&boundary), 0);
    assert!(payloads(&boundary, "memory::correct").is_empty());
    assert_eq!(bank.verify_skipped, 1);
    assert_eq!(bank.notes, ["verify: skipped 1 (session_not_found 1)"]);
    assert!(bank.errors.is_empty());
    assert!(response.checkpointed);
    assert_eq!(
        verify_cursor(&boundary)["first"],
        json!({"updated_at":1,"id":"tea"})
    );
}

#[tokio::test]
async fn verify_skips_unspanned_and_reinforced_notes_without_any_call() {
    let mut reinforced = tea("extracted");
    reinforced["corroboration"] = json!(1);
    reinforced["updated_at"] = json!(2);
    let mut pinned = standup();
    pinned["pinned"] = json!(true);
    let boundary = Boundary::with_rows(vec![
        row("legacy", "User lives in Lisbon.", 1),
        reinforced,
        pinned,
    ]);
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert!(payloads(&boundary, "memory::transcript").is_empty());
    assert_eq!(model_calls(&boundary), 0);
    assert_eq!(bank.verify_skipped, 2);
    assert_eq!(
        bank.notes,
        ["verify: skipped 2 (no recorded span 1, reinforced 1)"]
    );
    // Local skips are final: the cursor passes them in one save.
    assert_eq!(
        verify_cursor(&boundary)["rest"],
        json!({"updated_at":2,"id":"tea"})
    );
    let saves = payloads(&boundary, "state::set");
    assert_eq!(
        saves
            .iter()
            .filter(|p| p["key"] == "verify_cursor:offline")
            .count(),
        1
    );
}

#[tokio::test]
async fn verify_stops_with_one_note_on_an_older_memory_worker() {
    let older = "memory::transcript: remote error (function_not_found): function not found";
    // Memory is current but session-manager is missing: a plain stop.
    let no_sessions = "memory::transcript: remote error (invocation_failed): sibling_unavailable: \
                       session::messages: remote error (function_not_found): function not found";
    for (error, note) in [
        (
            older,
            "verify tier skipped: memory worker lacks memory::transcript — upgrade memory"
                .to_string(),
        ),
        (no_sessions, format!("verify tier stopped: {no_sessions}")),
    ] {
        let boundary = with_transcripts(vec![tea("inferred"), standup()]);
        boundary.0.lock().unwrap().transcript_error = Some(error.into());
        let response = run(&boundary, verify_only(), false).await;
        let bank = &response.banks[0];
        assert_eq!(payloads(&boundary, "memory::transcript").len(), 1);
        assert_eq!(model_calls(&boundary), 0);
        assert!(verify_cursor(&boundary).is_null());
        assert!(bank.errors.is_empty(), "{bank:?}");
        assert_eq!(bank.notes, [note]);
    }
}

#[tokio::test]
async fn verify_refused_from_an_agent_turn_keeps_the_pass_due() {
    let refused = "memory::transcript: remote error (invocation_failed): forbidden: calling \
                   `memory::transcript` is not allowed from inside an agent turn; it is a \
                   console, REST or CLI action";
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    boundary.0.lock().unwrap().transcript_error = Some(refused.into());
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(model_calls(&boundary), 0);
    assert!(verify_cursor(&boundary).is_null());
    assert_eq!(bank.errors, [format!("verify tier stopped: {refused}")]);
    assert!(!response.checkpointed);
}

#[tokio::test]
async fn verify_checks_priority_notes_first() {
    let boundary = with_transcripts(vec![tea("extracted"), standup()]);
    boundary.decision(json!({"verdict":"supported"}));
    let cfg = WorkerConfig {
        verify_max_per_run: 1,
        ..verify_only()
    };
    let response = run(&boundary, cfg, false).await;
    assert_eq!(
        payloads(&boundary, "memory::transcript"),
        [json!({"bank":"offline","id":"standup"})]
    );
    assert_eq!(response.banks[0].verified, 1);
    assert_eq!(
        verify_cursor(&boundary),
        json!({"first":{"updated_at":2,"id":"standup"},"rest":{"updated_at":0,"id":""}})
    );
}

#[tokio::test]
async fn verify_defers_at_the_llm_call_cap() {
    let boundary = with_transcripts(vec![tea("inferred")]);
    let cfg = WorkerConfig {
        max_llm_calls_per_run: 0,
        ..verify_only()
    };
    let response = run(&boundary, cfg, false).await;
    assert!(payloads(&boundary, "memory::transcript").is_empty());
    assert_eq!(model_calls(&boundary), 0);
    assert_eq!(
        response.banks[0].notes,
        ["verify tier deferred: per-run LLM call cap reached"]
    );
    assert!(verify_cursor(&boundary).is_null());
}

#[tokio::test]
async fn verify_makes_no_calls_on_a_dry_run() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    let before = boundary.snapshot();
    let response = run(&boundary, verify_only(), true).await;
    assert!(payloads(&boundary, "memory::transcript").is_empty());
    assert!(payloads(&boundary, "memory::correct").is_empty());
    assert_eq!(model_calls(&boundary), 0);
    assert!(
        response.banks[0]
            .notes
            .iter()
            .any(|n| n.contains("unevaluated"))
    );
    assert_eq!(before, boundary.snapshot());
}

#[tokio::test]
async fn verify_holds_the_cursor_when_no_reply_parses() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    boundary.decision(json!({"answer":"looks fine"}));
    boundary.decision(json!({"answer":"looks fine"}));
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(model_calls(&boundary), 2);
    assert!(payloads(&boundary, "memory::correct").is_empty());
    assert!(verify_cursor(&boundary).is_null());
    assert!(
        bank.notes
            .iter()
            .any(|n| n.contains("cursor held before `tea`")),
        "{:?}",
        bank.notes
    );
}

#[tokio::test]
async fn verify_holds_the_cursor_when_every_reply_is_truncated() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    for _ in 0..2 {
        boundary.0.lock().unwrap().replies.push_back(Ok(json!({
            "message":{"stop_reason":"length","content":[{"type":"text","text":"{\"verdict\":"}]}
        })));
    }
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(model_calls(&boundary), 2);
    assert!(payloads(&boundary, "memory::correct").is_empty());
    assert!(verify_cursor(&boundary).is_null());
    assert_eq!(bank.verify_rejected, 0);
    assert!(
        bank.notes
            .iter()
            .any(|n| n.contains("cursor held before `tea`")),
        "{:?}",
        bank.notes
    );
}

#[tokio::test]
async fn verify_passes_a_held_priority_note_once_the_other_lane_parses() {
    let plain = spanned(
        "standup",
        "User's team standup is at 10am.",
        2,
        "extracted",
        "s-standup",
    );
    let boundary = with_transcripts(vec![tea("inferred"), plain]);
    boundary.decision(json!({"answer":"looks fine"}));
    boundary.decision(json!({"verdict":"supported"}));
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!((bank.verify_rejected, bank.verified), (1, 1));
    assert!(bank.notes.is_empty(), "{:?}", bank.notes);
    assert_eq!(
        verify_cursor(&boundary),
        json!({"first":{"updated_at":1,"id":"tea"},"rest":{"updated_at":2,"id":"standup"}})
    );
    // Passed for good: the next pass judges nothing.
    run(&boundary, verify_only(), false).await;
    assert_eq!(model_calls(&boundary), 2);
}

#[tokio::test]
async fn verify_passes_an_unparsed_note_once_a_later_reply_parses() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    boundary.decision(json!({"answer":"looks fine"}));
    boundary.decision(json!({"verdict":"supported"}));
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!((bank.verify_rejected, bank.verified), (1, 1));
    assert!(bank.notes.is_empty(), "{:?}", bank.notes);
    assert_eq!(
        verify_cursor(&boundary)["first"],
        json!({"updated_at":2,"id":"standup"})
    );
}

#[tokio::test]
async fn verify_saves_the_cursor_after_each_note() {
    let boundary = with_transcripts(vec![tea("inferred"), standup()]);
    // No second reply: the second judge call fails as transport.
    boundary.decision(json!({"verdict":"supported"}));
    let response = run(&boundary, verify_only(), false).await;
    let bank = &response.banks[0];
    assert_eq!(model_calls(&boundary), 2);
    assert_eq!(bank.verified, 1);
    assert_eq!(
        bank.notes,
        ["verify tier stopped: no synthetic decision supplied"]
    );
    assert_eq!(
        verify_cursor(&boundary)["first"],
        json!({"updated_at":1,"id":"tea"})
    );
}

// ---- alias tier ----------------------------------------------------------

fn tagged(id: &str, text: &str, at: u64, handles: &[&str]) -> Value {
    let mut memory = row(id, text, at);
    memory["entities"] = json!(handles);
    memory
}

/// `build-cache` on two memories, `build cache` on one.
fn build_cache() -> Vec<Value> {
    vec![
        tagged(
            "a",
            "The build cache lives on the CI runner.",
            1,
            &["build-cache"],
        ),
        tagged(
            "b",
            "The build cache is pruned weekly.",
            2,
            &["build-cache", "ci"],
        ),
        tagged(
            "c",
            "Clear the build cache before a release.",
            3,
            &["user", "build cache"],
        ),
    ]
}

/// `dentist` and `dentista`, one memory each: a near pair.
fn dentist() -> Vec<Value> {
    vec![
        tagged(
            "d",
            "The user's dentist is Dr. Silva.",
            4,
            &["user", "dentist"],
        ),
        tagged(
            "e",
            "The dentista appointment moved to Friday.",
            5,
            &["user", "dentista"],
        ),
    ]
}

fn same(reason: &str) -> Value {
    json!({"pairs":[{"n":1,"same":true,"reason":reason}]})
}

fn entities_of(boundary: &Boundary, id: &str) -> Value {
    boundary.snapshot()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()["entities"]
        .clone()
}

/// The ledger without its clock.
fn ledger(boundary: &Boundary) -> Value {
    let mut entries = boundary.snapshot()["state"]["aliases:offline"].clone();
    for entry in entries.as_array_mut().into_iter().flatten() {
        entry.as_object_mut().unwrap().remove("ms");
    }
    entries
}

#[tokio::test]
async fn a_dry_run_lists_alias_candidates_and_writes_nothing() {
    let boundary = Boundary::with_rows([build_cache(), dentist()].concat());
    boundary.decision(same("never asked"));
    let before = boundary.snapshot();
    let response = run(&boundary, deterministic_only(), true).await;
    assert_eq!(
        serde_json::to_value(&response.banks[0].aliases).unwrap(),
        json!([
            {"from":"build cache","to":"build-cache","tier":"spelling","similarity":1.0,
             "applied":false,"ids":["c"],"reason":"same letters, separators aside"},
            {"from":"dentista","to":"dentist","tier":"near","similarity":0.7,
             "applied":false,"reason":"waits for the judge"},
        ])
    );
    assert_eq!(before, boundary.snapshot());
    assert!(
        boundary
            .calls()
            .iter()
            .all(|(f, _)| f != "memory::update" && f != "router::complete" && f != "state::set")
    );
}

#[tokio::test]
async fn a_spelling_alias_is_retagged_without_a_model_call() {
    let rows = build_cache();
    let boundary = Boundary::with_rows(rows.clone());
    boundary.0.lock().unwrap().guarded = true;
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    let mut expected = rows[2].clone();
    expected["stated_at"] = Value::Null;
    assert_eq!(
        payloads(&boundary, "memory::update"),
        [json!({
            "bank": "offline",
            "id": "c",
            "entities": ["user", "build-cache"],
            "expected_memory": expected,
            "reason": "alias: build cache -> build-cache (same letters, separators aside)",
        })]
    );
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build-cache"]));
    assert_eq!(model_calls(&boundary), 0);
    let aliases = &response.banks[0].aliases;
    assert!(aliases[0].applied);
    assert_eq!(aliases[0].ids, ["c"]);
    assert_eq!(
        ledger(&boundary),
        json!([{"from":"build cache","to":"build-cache","tier":"spelling","verdict":"same",
            "reason":"same letters, separators aside","ids":["c"]}])
    );

    let again = run(&boundary, deterministic_only(), false).await;
    assert!(again.banks[0].aliases.is_empty());
    assert_eq!(payloads(&boundary, "memory::update").len(), 1);
}

#[tokio::test]
async fn a_near_alias_waits_for_the_judge() {
    // No router: the pair waits with a note, nothing is written.
    let boundary = Boundary::with_rows(dentist());
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(
        response.checkpointed,
        "a missing router never blocks the pass"
    );
    assert_eq!(response.banks[0].aliases[0].reason, "waits for the judge");
    assert_eq!(
        response.banks[0].notes,
        ["alias judge skipped: no synthetic decision supplied"]
    );
    assert!(payloads(&boundary, "memory::update").is_empty());

    // The call cap at 0: no call at all.
    let capped = WorkerConfig {
        max_llm_calls_per_run: 0,
        ..deterministic_only()
    };
    let response = run(&boundary, capped, false).await;
    assert_eq!(model_calls(&boundary), 1);
    assert_eq!(
        response.banks[0].notes,
        ["alias judge deferred: per-run LLM call cap reached"]
    );

    // `same`: retagged, with the judge's reason.
    boundary.decision(same("another language's word for the same dentist"));
    let response = run(&boundary, deterministic_only(), false).await;
    assert_eq!(model_calls(&boundary), 2);
    let prompt = &payloads(&boundary, "router::complete")[1]["prompt"];
    let prompt: Value = serde_json::from_str(prompt.as_str().unwrap()).unwrap();
    assert_eq!(
        prompt,
        json!({"pairs":[{"n":1,"a":"dentista","a_memories":["The dentista appointment moved to Friday."],
            "b":"dentist","b_memories":["The user's dentist is Dr. Silva."]}]})
    );
    let updates = payloads(&boundary, "memory::update");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["entities"], json!(["user", "dentist"]));
    assert_eq!(
        updates[0]["reason"],
        "alias: dentista -> dentist (another language's word for the same dentist)"
    );
    assert!(response.banks[0].aliases[0].applied);

    // `distinct`: nothing written, and the pair is never asked again.
    let boundary = Boundary::with_rows(vec![
        tagged("g1", "Gabriel runs the payments on-call.", 1, &["gabriel"]),
        tagged("g2", "Gabriela joined the design team.", 2, &["gabriela"]),
    ]);
    boundary.decision(json!({"pairs":[{"n":1,"same":false,"reason":"two people"}]}));
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(!response.banks[0].aliases[0].applied);
    assert_eq!(response.banks[0].aliases[0].reason, "two people");
    let again = run(&boundary, deterministic_only(), false).await;
    assert!(again.banks[0].aliases.is_empty());
    assert_eq!(model_calls(&boundary), 1);
    assert!(payloads(&boundary, "memory::update").is_empty());
    assert_eq!(ledger(&boundary)[0]["verdict"], "distinct");
}

#[tokio::test]
async fn payments_api_and_ledger_api_are_never_offered_to_the_judge() {
    // The seeds of harness-e2e's linked-update case.
    let boundary = Boundary::with_rows(vec![
        tagged(
            "on",
            "dana-1 is the primary on-call engineer for payments-api incidents.",
            1,
            &["dana", "payments-api"],
        ),
        tagged(
            "up",
            "Unanswered payments-api pages escalate to marco-2 after 15 minutes.",
            2,
            &["marco", "payments-api"],
        ),
        tagged(
            "near",
            "kai-3 is the primary on-call engineer for ledger-api incidents.",
            3,
            &["kai", "ledger-api"],
        ),
        tagged("left", "dana-1 left our company last week.", 4, &["dana"]),
    ]);
    let yes: Vec<Value> = (1..=20)
        .map(|n| json!({"n":n,"same":true,"reason":"same"}))
        .collect();
    boundary.decision(json!({ "pairs": yes }));
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.banks[0].aliases.is_empty());
    assert_eq!(model_calls(&boundary), 0);
    assert!(payloads(&boundary, "memory::update").is_empty());
    assert!(
        payloads(&boundary, "state::get")
            .iter()
            .all(|p| p["key"] != "aliases:offline")
    );
}

#[tokio::test]
async fn retags_share_the_supersede_cap_and_finish_on_the_next_pass() {
    let boundary = Boundary::with_rows(vec![
        tagged("d1", "The dentist is on Main Street.", 1, &["dentist"]),
        tagged("d2", "The dentist opens at nine.", 2, &["dentist"]),
        tagged(
            "e1",
            "The dentista appointment is on Friday.",
            3,
            &["dentista"],
        ),
        tagged("e2", "The dentista sent a reminder.", 4, &["dentista"]),
    ]);
    boundary.decision(same("the same dentist"));
    let cfg = WorkerConfig {
        max_supersedes_per_run: 1,
        ..deterministic_only()
    };
    let response = run(&boundary, cfg.clone(), false).await;
    assert_eq!(response.banks[0].capped, 1);
    assert_eq!(response.banks[0].aliases[0].ids, ["e1"]);
    assert_eq!(ledger(&boundary)[0]["ids"], json!(["e1"]));

    let response = run(&boundary, cfg.clone(), false).await;
    assert_eq!(response.banks[0].capped, 0);
    assert_eq!(response.banks[0].aliases[0].ids, ["e2"]);
    assert_eq!(response.banks[0].aliases[0].reason, "the same dentist");
    assert_eq!(model_calls(&boundary), 1, "a recorded `same` needs no call");
    assert_eq!(ledger(&boundary)[0]["ids"], json!(["e1", "e2"]));
    assert_eq!(entities_of(&boundary, "e2"), json!(["dentist"]));

    assert!(run(&boundary, cfg, false).await.banks[0].aliases.is_empty());
    assert_eq!(payloads(&boundary, "memory::update").len(), 2);
}

#[tokio::test]
async fn a_failed_retag_keeps_the_pass_due() {
    let boundary = Boundary::with_rows(build_cache());
    boundary.fail_once("memory::update");
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(!response.checkpointed);
    assert!(!response.banks[0].errors.is_empty());
    assert!(!response.banks[0].aliases[0].applied);
    assert_eq!(boundary.snapshot()["state"]["last_run"], json!({"ms":7}));
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build cache"]));

    let retry = run(&boundary, deterministic_only(), false).await;
    assert!(retry.checkpointed);
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build-cache"]));
}

#[tokio::test]
async fn a_pinned_memory_keeps_its_handle() {
    let mut pinned = tagged(
        "p",
        "Never clear the build cache on Fridays.",
        4,
        &["build cache"],
    );
    pinned["pinned"] = json!(true);
    let boundary = Boundary::with_rows([build_cache(), vec![pinned]].concat());
    let response = run(&boundary, deterministic_only(), false).await;
    let alias = &response.banks[0].aliases[0];
    assert_eq!(
        (alias.from.as_str(), alias.to.as_str()),
        ("build cache", "build-cache")
    );
    assert_eq!(alias.ids, ["c"]);
    assert_eq!(alias.skipped_pinned, ["p"]);
    assert_eq!(entities_of(&boundary, "p"), json!(["build cache"]));
    assert_eq!(payloads(&boundary, "memory::update").len(), 1);
}

#[tokio::test]
async fn an_older_memory_worker_is_still_retagged() {
    // A record edited after the scan: memory that knows the guard refuses
    // the retag, and the pass stays due.
    let boundary = Boundary::with_rows(build_cache());
    boundary.0.lock().unwrap().guarded = true;
    boundary.0.lock().unwrap().stale = Some("c".into());
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(!response.checkpointed);
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build cache"]));

    // An older memory worker ignores `expected_memory` and `reason`: the
    // retag lands all the same.
    let boundary = Boundary::with_rows(build_cache());
    boundary.0.lock().unwrap().stale = Some("c".into());
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed);
    assert!(response.banks[0].aliases[0].applied);
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build-cache"]));
}

#[tokio::test]
async fn an_undone_merge_is_restored_and_never_merged_again() {
    use memory_consolidate::alias::UndoRequest;
    let boundary = Boundary::with_rows(build_cache());
    boundary.0.lock().unwrap().guarded = true;
    run(&boundary, deterministic_only(), false).await;
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build-cache"]));

    let deps = deps(deterministic_only());
    let undo = |from: &str| UndoRequest {
        bank: "offline".into(),
        from: from.into(),
    };
    let undone = functions::alias_undo_with_transport(&deps, undo("build cache"), &boundary)
        .await
        .unwrap();
    assert_eq!(
        (undone.to.as_str(), undone.restored.as_slice()),
        ("build-cache", &["c".to_string()][..])
    );
    assert_eq!(entities_of(&boundary, "c"), json!(["user", "build cache"]));
    let updates = payloads(&boundary, "memory::update");
    assert_eq!(
        updates[1]["reason"],
        "alias merge undone: build-cache -> build cache"
    );
    assert_eq!(ledger(&boundary)[0]["verdict"], "distinct");
    assert_eq!(ledger(&boundary)[0]["ids"], json!(["c"]));

    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed);
    assert!(response.banks[0].aliases.is_empty());
    assert_eq!(payloads(&boundary, "memory::update").len(), 2);

    // A second undo finds nothing left to restore; an unknown handle is an error.
    let again = functions::alias_undo_with_transport(&deps, undo("build cache"), &boundary)
        .await
        .unwrap();
    assert!(again.restored.is_empty());
    assert_eq!(again.skipped, ["c"]);
    let unknown = functions::alias_undo_with_transport(&deps, undo("dentista"), &boundary).await;
    assert!(
        unknown
            .unwrap_err()
            .to_string()
            .contains("no alias merge from `dentista`")
    );
}

#[tokio::test]
async fn a_merge_winner_is_retagged_on_the_next_pass() {
    // `w` absorbs its duplicate first, so this pass's snapshot of it is stale.
    let boundary = Boundary::with_rows(vec![
        tagged(
            "w",
            "Clear the build cache before a release.",
            1,
            &["build cache"],
        ),
        tagged(
            "w2",
            "clear the build cache before a release!",
            2,
            &["build cache"],
        ),
        tagged(
            "a",
            "The build cache lives on the CI runner.",
            3,
            &["build-cache"],
        ),
        tagged(
            "b",
            "The build cache is pruned weekly.",
            4,
            &["build-cache"],
        ),
    ]);
    boundary.0.lock().unwrap().guarded = true;
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert_eq!(response.superseded, 1);
    assert!(payloads(&boundary, "memory::update").is_empty());

    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert_eq!(response.banks[0].aliases[0].ids, ["w"]);
    assert_eq!(entities_of(&boundary, "w"), json!(["build-cache"]));
}

#[tokio::test]
async fn symbol_and_dot_variants_are_never_merged_without_the_judge() {
    let no_calls = WorkerConfig {
        max_llm_calls_per_run: 0,
        ..deterministic_only()
    };
    for (a, b) in [
        ("c++", "c#"),
        ("c", "c++"),
        ("java 1.8", "java 18"),
        ("angular 1.5", "angular 15"),
    ] {
        let boundary = Boundary::with_rows(vec![
            tagged("x", "The billing service is written in it.", 1, &[a]),
            tagged("y", "The build tooling targets it.", 2, &[b]),
        ]);
        let response = run(&boundary, no_calls.clone(), false).await;
        assert_eq!(model_calls(&boundary), 0, "{a} / {b}");
        assert!(
            payloads(&boundary, "memory::update").is_empty(),
            "{a} / {b}"
        );
        assert!(
            response.banks[0].aliases.iter().all(|p| !p.applied),
            "{a} / {b}"
        );
    }
}

/// Three near pairs, one memory per handle: `adriana`, `daniela` and
/// `dentista` against the older handle of each.
fn three_near_pairs() -> Vec<Value> {
    vec![
        tagged("a1", "Adrian reviews the payments code.", 1, &["adrian"]),
        tagged("a2", "Adriana leads the design review.", 2, &["adriana"]),
        tagged("d1", "Daniel owns the deploy scripts.", 3, &["daniel"]),
        tagged("d2", "Daniela joined the data team.", 4, &["daniela"]),
        tagged("t1", "The dentist is Dr. Silva.", 5, &["dentist"]),
        tagged(
            "t2",
            "The dentista appointment moved to Friday.",
            6,
            &["dentista"],
        ),
    ]
}

fn truncated(boundary: &Boundary) {
    boundary.0.lock().unwrap().replies.push_back(Ok(json!({
        "message":{"stop_reason":"length","content":[{"type":"text","text":"{\"pairs\":[{\"n\":1,"}]}
    })));
}

/// How many pairs each judge call was shown.
fn asked(boundary: &Boundary) -> Vec<usize> {
    payloads(boundary, "router::complete")
        .iter()
        .map(|p| {
            let prompt: Value = serde_json::from_str(p["prompt"].as_str().unwrap()).unwrap();
            prompt["pairs"].as_array().unwrap().len()
        })
        .collect()
}

fn verdicts(boundary: &Boundary) -> Vec<(String, String, String)> {
    ledger(boundary)
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let s = |k: &str| e[k].as_str().unwrap().to_string();
            (s("from"), s("verdict"), s("reason"))
        })
        .collect()
}

#[tokio::test]
async fn a_truncated_judge_reply_halves_the_batch_and_the_pass_makes_progress() {
    let boundary = Boundary::with_rows(three_near_pairs());
    truncated(&boundary);
    boundary.decision(json!({"pairs":[{"n":1,"same":false,"reason":"two people"}]}));
    boundary.decision(json!({"pairs":[
        {"n":1,"same":false,"reason":"two people"},
        {"n":2,"same":true,"reason":"the same dentist"}]}));
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert_eq!(asked(&boundary), [3, 1, 2]);
    assert_eq!(
        verdicts(&boundary),
        [
            ("adriana".into(), "distinct".into(), "two people".into()),
            ("daniela".into(), "distinct".into(), "two people".into()),
            ("dentista".into(), "same".into(), "the same dentist".into()),
        ]
    );
    assert_eq!(entities_of(&boundary, "t2"), json!(["dentist"]));
    // Every pair has its verdict: the next pass asks nothing.
    run(&boundary, deterministic_only(), false).await;
    assert_eq!(model_calls(&boundary), 3);
}

#[tokio::test]
async fn a_pair_whose_reply_never_parses_is_kept_apart_once_another_parses() {
    let boundary = Boundary::with_rows(three_near_pairs());
    // No reply parses at all: the judge itself fails, nothing is recorded.
    for _ in 0..5 {
        boundary.decision(json!("no verdict here"));
    }
    let response = run(&boundary, deterministic_only(), false).await;
    assert_eq!(asked(&boundary), [3, 1, 2, 1, 1]);
    assert!(ledger(&boundary).is_null(), "{}", ledger(&boundary));
    assert!(
        response.banks[0]
            .aliases
            .iter()
            .all(|p| p.reason == "waits for the judge")
    );

    // `daniela` never parses, alone or not; its siblings do.
    boundary.decision(json!("no verdict here"));
    boundary.decision(json!({"pairs":[{"n":1,"same":false,"reason":"two people"}]}));
    boundary.decision(json!("no verdict here"));
    boundary.decision(json!("no verdict here"));
    boundary.decision(json!({"pairs":[{"n":1,"same":true,"reason":"the same dentist"}]}));
    run(&boundary, deterministic_only(), false).await;
    assert_eq!(asked(&boundary)[5..], [3, 1, 2, 1, 1]);
    let kept_apart = verdicts(&boundary);
    assert_eq!(kept_apart[1].0, "daniela");
    assert_eq!(kept_apart[1].1, "distinct");
    assert_eq!(kept_apart.len(), 3);
    assert_eq!(entities_of(&boundary, "t2"), json!(["dentist"]));
    // It blocks nothing: the next pass asks nothing.
    run(&boundary, deterministic_only(), false).await;
    assert_eq!(model_calls(&boundary), 10);
}

#[tokio::test]
async fn user_is_never_retagged_by_a_merge_or_an_undo() {
    use memory_consolidate::alias::UndoRequest;
    // Seven handles plus `user`: the most a retag sends.
    let eight = [
        "build cache",
        "postgres",
        "redis",
        "kafka",
        "nginx",
        "docker",
        "rust",
        "user",
    ];
    let mut rows = build_cache();
    rows[2]["entities"] = json!(eight);
    let boundary = Boundary::with_rows(rows);
    run(&boundary, deterministic_only(), false).await;
    let mut merged = eight;
    merged[0] = "build-cache";
    assert_eq!(entities_of(&boundary, "c"), json!(merged));
    let deps = deps(deterministic_only());
    let undo = |from: &str| UndoRequest {
        bank: "offline".into(),
        from: from.into(),
    };
    functions::alias_undo_with_transport(&deps, undo("build cache"), &boundary)
        .await
        .unwrap();
    assert_eq!(entities_of(&boundary, "c"), json!(eight));

    // A ledger entry naming `user`, written by hand, is never undone.
    let entry = |from: &str, to: &str| {
        json!({"from":from,"to":to,"tier":"spelling","verdict":"same","reason":"by hand",
            "ms":1,"ids":["c"]})
    };
    boundary.0.lock().unwrap().state.insert(
        "aliases:offline".into(),
        json!([entry("user", "users"), entry("me", "user")]),
    );
    for from in ["user", "me"] {
        let refused = functions::alias_undo_with_transport(&deps, undo(from), &boundary).await;
        assert!(refused.is_err(), "{from}");
    }
    assert_eq!(entities_of(&boundary, "c"), json!(eight));
    assert_eq!(payloads(&boundary, "memory::update").len(), 2);
}

#[tokio::test]
async fn a_memory_with_more_than_eight_handles_is_never_retagged() {
    use memory_consolidate::alias::UndoRequest;
    // Eight handles plus `user`, as extraction writes them: an older memory
    // worker cuts an update at eight, so a retag would drop `user`.
    let nine = [
        "build cache",
        "postgres",
        "redis",
        "kafka",
        "nginx",
        "docker",
        "rust",
        "tokio",
        "user",
    ];
    let mut rows = build_cache();
    rows[2]["entities"] = json!(nine);
    let boundary = Boundary::with_rows(rows);
    let response = run(&boundary, deterministic_only(), false).await;
    assert!(response.checkpointed, "{:?}", response.banks[0].errors);
    assert!(payloads(&boundary, "memory::update").is_empty());
    assert_eq!(entities_of(&boundary, "c"), json!(nine));
    assert!(!response.banks[0].aliases[0].applied);
    assert_eq!(
        notes_with(&response.banks[0], "more than 8 handles"),
        1,
        "{:?}",
        response.banks[0].notes
    );

    // An undo leaves such a memory alone too.
    let mut merged = nine;
    merged[0] = "build-cache";
    boundary.0.lock().unwrap().rows[2]["entities"] = json!(merged);
    boundary.0.lock().unwrap().state.insert(
        "aliases:offline".into(),
        json!([{"from":"build cache","to":"build-cache","tier":"spelling","verdict":"same",
            "reason":"same letters, separators aside","ms":1,"ids":["c"]}]),
    );
    let undone = functions::alias_undo_with_transport(
        &deps(deterministic_only()),
        UndoRequest {
            bank: "offline".into(),
            from: "build cache".into(),
        },
        &boundary,
    )
    .await
    .unwrap();
    assert_eq!(undone.skipped, ["c"]);
    assert!(payloads(&boundary, "memory::update").is_empty());
    assert_eq!(entities_of(&boundary, "c"), json!(merged));
}
