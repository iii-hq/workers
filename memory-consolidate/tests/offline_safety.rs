//! Offline transport-contract tests. Supplied decisions are synthetic;
//! replaying them does not measure a model's semantic quality.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use memory_consolidate::config::WorkerConfig;
use memory_consolidate::consolidate::{self, ConsolidationTransport, MemoryRow, TransportFuture};
use memory_consolidate::functions::{self, Deps, RunRequest};
use memory_consolidate::semantic::{self, SemanticOptions};
use serde_json::{json, Value};

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
                ("last_report".into(), json!({"previous":true})),
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
                    let hits: Vec<Value> = inner
                        .rows
                        .iter()
                        .filter(|r| r["invalid_at"].is_null())
                        .map(|r| json!({"memory":r}))
                        .collect();
                    Ok(json!({"retrieval":payload["mode"],"memories":hits}))
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
                _ => Err(format!("unexpected offline RPC: {function}")),
            }
        })
    }

    fn judge<'a>(
        &'a self,
        _model: &'a str,
        prompt: String,
    ) -> TransportFuture<'a, memory_consolidate::llm::JudgeReply> {
        Box::pin(async move {
            let mut inner = self.0.lock().unwrap();
            inner
                .calls
                .push(("offline::reorder-decision".into(), json!({"prompt":prompt})));
            let reply = inner
                .replies
                .pop_front()
                .unwrap_or_else(|| Err("no synthetic reorder decision supplied".into()))?;
            Ok(memory_consolidate::llm::parse_reply(
                reply.as_str().unwrap(),
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
    assert!(response.banks[0]
        .notes
        .iter()
        .any(|n| n.contains("unevaluated")));
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
    assert_eq!(after["state"]["last_report"]["dry_run"], true);
    assert_eq!(
        after["state"]["last_report"]["banks"][0]["groups"][0]["loser_ids"],
        json!(["b"])
    );
    assert_ne!(before["state"]["last_run"], after["state"]["last_run"]);
    assert_eq!(
        boundary
            .calls()
            .iter()
            .map(|(f, p)| format!("{f} {}", p["key"].as_str().unwrap_or("")))
            .collect::<Vec<_>>(),
        [
            "memory::list ",
            "state::set last_report",
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
    assert!(boundary
        .calls()
        .iter()
        .all(|(f, _)| !f.starts_with("memory::rule::")));
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
    assert!(!boundary
        .calls()
        .iter()
        .any(|(f, p)| f == "state::set" && p["key"] == "last_run"));
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
        assert!(boundary
            .calls()
            .iter()
            .all(|(f, _)| f != "router::models::list"
                && f != "router::embed"
                && !f.starts_with("memory::rule::")));
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
        assert!(!boundary
            .calls()
            .iter()
            .any(|(f, p)| f == "state::set" && p["key"] == "last_run"));
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
        assert!(boundary
            .calls()
            .iter()
            .all(|(f, _)| f != "memory::supersede"));
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
    assert!(failed
        .notes
        .iter()
        .any(|n| n.contains("no synthetic decision supplied")));
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
    assert!(boundary
        .calls()
        .iter()
        .all(|(f, _)| f != "router::models::list" && f != "router::embed"));
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
                assert!(!boundary
                    .calls()
                    .iter()
                    .any(|(f, _)| f == "router::complete"));
            }
            "memory::supersede" => {
                assert_eq!(budget, 1);
                assert_eq!(report.superseded, 0);
                assert!(!report.errors.is_empty());
                assert!(boundary.snapshot()["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["invalid_at"].is_null()));
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
        assert!(boundary.snapshot()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["invalid_at"].is_null()));
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
    assert!(boundary.snapshot()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["invalid_at"].is_null()));
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
