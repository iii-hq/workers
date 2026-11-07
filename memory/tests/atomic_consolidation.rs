//! Offline producer/consumer contract: actual handlers and filesystem, no CAS mock.
//! Compile the consumer's existing source without adding a dependency/lock change.
// The source modules also expose standalone worker entry points not used here.
#![allow(dead_code)]
#[path = "../../memory-consolidate/src/config.rs"]
mod config;
#[path = "../../memory-consolidate/src/consolidate.rs"]
mod consolidate;
#[path = "../../memory-consolidate/src/functions.rs"]
mod functions;
#[path = "../../memory-consolidate/src/llm.rs"]
mod llm;
#[path = "../../memory-consolidate/src/semantic.rs"]
mod semantic;

use consolidate::{ConsolidationTransport, TransportFuture};
use memory::{
    deps::Deps,
    events::Emitter,
    functions::items,
    store::Store,
    types::{Confidence, Memory, Provenance},
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn record(id: &str) -> Memory {
    Memory {
        id: id.into(),
        text: "An exact durable fact.".into(),
        entities: vec!["subject".into()],
        tags: vec!["durable".into()],
        confidence: Confidence::Stated,
        corroboration: 2,
        pinned: false,
        source: Some(Provenance {
            session_id: Some("synthetic".into()),
            entry_id: Some(id.into()),
            agent: None,
        }),
        created_at: 1,
        updated_at: 2,
        stated_at: Some(2),
        invalid_at: None,
        superseded_by: None,
        revision: 0,
    }
}

async fn fixture() -> (tempfile::TempDir, Deps) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path().into()).unwrap());
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    for id in ["a", "b", "c"] {
        bank.commit(record(id)).await.unwrap();
    }
    bank.set_rule("permanent", "Permanent rule; preserve bytes.\n")
        .unwrap();
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
    let cfg = memory::config::WorkerConfig {
        embeddings_enabled: false,
        extraction_enabled: false,
        ..Default::default()
    };
    let deps = Deps {
        emitter: Arc::new(Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(store)),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
    };
    (dir, deps)
}

fn request(
    expected_loser: Option<Value>,
    expected_winner: Option<Value>,
) -> items::SupersedeRequest {
    serde_json::from_value(json!({"bank":"offline","id":"b","superseded_by":"a","reinforce":true,
        "expected_memory":expected_loser,"expected_superseded_by":expected_winner,"reason":"exact byte equality"})).unwrap()
}

struct ActualBoundary {
    deps: Deps,
    calls: Mutex<Vec<(String, Value)>>,
    interleave: Mutex<Option<(String, String)>>,
}

impl ConsolidationTransport for ActualBoundary {
    fn invoke<'a>(&'a self, function: &'a str, payload: Value) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((function.into(), payload.clone()));
            if function == "memory::supersede" {
                let interleave = self.interleave.lock().unwrap().take();
                if let Some((id, kind)) = interleave {
                    match kind.as_str() {
                        "text" => {
                            items::update(&self.deps, serde_json::from_value(json!({"bank":"offline","id":id,"text":"A genuinely changed fact."})).unwrap())
                                .await.map_err(|e| e.to_string())?;
                        }
                        "edit" => {
                            items::update(
                                &self.deps,
                                serde_json::from_value(
                                    json!({"bank":"offline","id":id,"tags":["changed"]}),
                                )
                                .unwrap(),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        }
                        "pin" => {
                            items::pin(
                                &self.deps,
                                serde_json::from_value(
                                    json!({"bank":"offline","id":id,"pinned":true}),
                                )
                                .unwrap(),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        }
                        "retire" => {
                            items::delete(
                                &self.deps,
                                serde_json::from_value(json!({"bank":"offline","id":id})).unwrap(),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            match function {
                "memory::list" => serde_json::to_value(
                    items::list(&self.deps, serde_json::from_value(payload).unwrap())
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string()),
                "memory::get" => serde_json::to_value(
                    items::get(&self.deps, serde_json::from_value(payload).unwrap())
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string()),
                "memory::supersede" => serde_json::to_value(
                    items::supersede(&self.deps, serde_json::from_value(payload).unwrap())
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string()),
                _ => Err(format!("unexpected dispatch: {function}")),
            }
        })
    }
}

#[tokio::test]
async fn atomic_consumer_positive_absorbs_each_once_and_replays_legacy_history() {
    let (dir, deps) = fixture().await;
    let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    let boundary = ActualBoundary {
        deps,
        calls: Mutex::new(Vec::new()),
        interleave: Mutex::new(None),
    };
    let (report, _) = consolidate::run_bank(&boundary, "offline", false, &mut 10).await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!((report.superseded, report.reinforced), (2, 2));
    {
        let calls = boundary.calls.lock().unwrap();
        let mutations: Vec<_> = calls
            .iter()
            .filter(|(f, _)| f == "memory::supersede")
            .collect();
        assert_eq!(mutations.len(), 2);
        assert_eq!(mutations[0].1["expected_memory"]["revision"], 0);
        assert_eq!(mutations[0].1["expected_superseded_by"]["revision"], 0);
        assert_eq!(mutations[1].1["expected_superseded_by"]["revision"], 1);
    }
    let after = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    assert!(after.starts_with(&before));
    assert_eq!(after.iter().filter(|&&b| b == b'\n').count(), 7);
    let reopened = Store::open(dir.path().into()).unwrap();
    let bank = reopened.bank("offline").await.unwrap();
    let winner = bank.get("a").await.unwrap();
    assert_eq!((winner.corroboration, winner.revision), (8, 2));
    for id in ["b", "c"] {
        let retired = bank.get(id).await.unwrap();
        assert!(!retired.is_live());
        assert_eq!(retired.superseded_by.as_deref(), Some("a"));
        assert_eq!(retired.source, record(id).source);
        assert_eq!(retired.text, record(id).text);
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("offline/rules/permanent.md")).unwrap(),
        "Permanent rule; preserve bytes.\n"
    );
    let retried = items::supersede(&boundary.deps, request(None, None)).await;
    assert!(retried.is_err());
    assert_eq!(
        std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
        after
    );
}

#[tokio::test]
async fn atomic_consumer_rejects_edit_pin_retire_on_either_proof_bound_record() {
    for id in ["a", "b"] {
        for kind in ["text", "edit", "pin", "retire"] {
            let (dir, deps) = fixture().await;
            let boundary = ActualBoundary {
                deps,
                calls: Mutex::new(Vec::new()),
                interleave: Mutex::new(Some((id.into(), kind.into()))),
            };
            let (report, _) = consolidate::run_bank(&boundary, "offline", false, &mut 1).await;
            assert_eq!(report.superseded, 0, "{id} {kind}");
            assert!(!report.errors.is_empty(), "{id} {kind}");
            let store = Store::open(dir.path().into()).unwrap();
            let bank = store.bank("offline").await.unwrap();
            for record_id in ["a", "b", "c"] {
                let m = bank.get(record_id).await.unwrap();
                assert_eq!(m.corroboration, 2, "no absorption after {id} {kind}");
                assert_eq!(m.revision, u64::from(record_id == id));
                assert!(m.superseded_by.is_none());
            }
            let lines = std::fs::read_to_string(dir.path().join("offline/memories.jsonl")).unwrap();
            assert_eq!(
                lines.lines().count(),
                4,
                "only the interleaved edit may write"
            );
        }
    }
}

#[tokio::test]
async fn atomic_storage_failure_retry_and_legacy_clients_preserve_both_records() {
    let (dir, deps) = fixture().await;
    let store = deps.store().await;
    let bank = store.bank("offline").await.unwrap();
    let loser = bank.get("b").await.unwrap();
    let winner = bank.get("a").await.unwrap();
    let path = dir.path().join("offline/memories.jsonl");
    let before = std::fs::read(&path).unwrap();
    let blocker = dir.path().join("offline/.memories.jsonl.tmp");
    std::fs::create_dir(&blocker).unwrap();
    let expected = || request(Some(json!(loser)), Some(json!(winner)));
    assert!(items::supersede(&deps, expected()).await.is_err());
    assert_eq!(bank.get("a").await.unwrap(), winner);
    assert_eq!(bank.get("b").await.unwrap(), loser);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir(&blocker).unwrap(); // synthetic failure fixture only
    let reply = items::supersede(&deps, expected()).await.unwrap();
    assert!(reply.reinforced);
    assert_eq!(reply.survivor.unwrap().corroboration, 5);
    let after = std::fs::read(&path).unwrap();
    assert!(items::supersede(&deps, expected()).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), after);
    // Legacy clients omit both new fields, but still use the same joint write.
    let legacy: items::SupersedeRequest = serde_json::from_value(
        json!({"bank":"offline","id":"c","superseded_by":"a","reinforce":true}),
    )
    .unwrap();
    assert!(items::supersede(&deps, legacy).await.unwrap().reinforced);
    let reopened = Store::open(dir.path().into()).unwrap();
    assert_eq!(
        reopened
            .bank("offline")
            .await
            .unwrap()
            .get("a")
            .await
            .unwrap()
            .corroboration,
        8
    );
}

#[tokio::test]
async fn atomic_competing_supersedes_have_one_winner_and_one_absorption() {
    let (dir, deps) = fixture().await;
    let bank = deps.store().await.bank("offline").await.unwrap();
    let loser = bank.get("b").await.unwrap();
    let winner = bank.get("a").await.unwrap();
    let expected = || request(Some(json!(loser)), Some(json!(winner)));
    let (a, b) = tokio::join!(
        items::supersede(&deps, expected()),
        items::supersede(&deps, expected())
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(bank.get("a").await.unwrap().corroboration, 5);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("offline/memories.jsonl"))
            .unwrap()
            .lines()
            .count(),
        5
    );
}

#[tokio::test]
async fn atomic_consumer_dry_run_preserves_actual_log_rules_and_dispatches() {
    let (dir, deps) = fixture().await;
    let path = dir.path().join("offline/memories.jsonl");
    let before = std::fs::read(&path).unwrap();
    let boundary = ActualBoundary {
        deps,
        calls: Mutex::new(Vec::new()),
        interleave: Mutex::new(None),
    };
    let deps = functions::Deps {
        iii: boundary.deps.iii.clone(),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(config::WorkerConfig {
            semantic_enabled: true,
            llm_assist_enabled: true,
            ..Default::default()
        }))),
        run_gate: tokio::sync::Mutex::new(()),
    };
    let report = functions::run_with_transport(
        &deps,
        functions::RunRequest {
            bank: Some("offline".into()),
            dry_run: Some(true),
        },
        &boundary,
    )
    .await
    .unwrap();
    assert_eq!(report.superseded, 0);
    assert!(!report.checkpointed);
    assert!(!report.banks[0].groups.is_empty(), "non-vacuous planning");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(boundary
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(f, _)| f == "memory::list"));
}

#[tokio::test]
async fn atomic_optional_snapshots_each_reject_changes_without_a_single_log_byte() {
    for (loser_guard, winner_guard) in [(true, false), (false, true), (true, true)] {
        let (dir, deps) = fixture().await;
        let bank = deps.store().await.bank("offline").await.unwrap();
        let loser = bank.get("b").await.unwrap();
        let winner = bank.get("a").await.unwrap();
        let changed_id = if loser_guard { "b" } else { "a" };
        items::pin(
            &deps,
            serde_json::from_value(json!({"bank":"offline","id":changed_id,"pinned":true}))
                .unwrap(),
        )
        .await
        .unwrap();
        let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
        assert!(items::supersede(
            &deps,
            request(
                loser_guard.then(|| json!(loser)),
                winner_guard.then(|| json!(winner))
            )
        )
        .await
        .is_err());
        assert_eq!(
            std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
            before
        );
        assert_eq!(bank.get("a").await.unwrap().corroboration, 2);
        assert!(bank.get("b").await.unwrap().is_live());
    }
}

#[tokio::test]
async fn atomic_pinned_survivor_is_untouched_and_unknown_snapshot_fields_fail_closed() {
    let (dir, deps) = fixture().await;
    let bank = deps.store().await.bank("offline").await.unwrap();
    items::pin(
        &deps,
        serde_json::from_value(json!({"bank":"offline","id":"a","pinned":true})).unwrap(),
    )
    .await
    .unwrap();
    let winner = bank.get("a").await.unwrap();
    let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    for invalid in [
        json!({}),
        json!({"unknown":null}),
        json!({"revision":999}),
        json!(7),
    ] {
        assert!(items::supersede(&deps, request(None, Some(invalid)))
            .await
            .is_err());
        assert_eq!(
            std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
            before
        );
    }
    let reply = items::supersede(&deps, request(None, Some(json!(winner))))
        .await
        .unwrap();
    assert!(!reply.reinforced);
    assert!(reply.survivor.is_none());
    assert_eq!(bank.get("a").await.unwrap(), winner);
    let reopened = Store::open(dir.path().into()).unwrap();
    assert_eq!(
        reopened
            .bank("offline")
            .await
            .unwrap()
            .get("a")
            .await
            .unwrap(),
        winner
    );
}

#[tokio::test]
async fn atomic_old_read_cannot_resurrect_retirement_or_overwrite_absorption() {
    let (dir, deps) = fixture().await;
    let bank = deps.store().await.bank("offline").await.unwrap();
    let loser = bank.get("b").await.unwrap();
    let winner = bank.get("a").await.unwrap();
    items::supersede(&deps, request(Some(json!(loser)), Some(json!(winner))))
        .await
        .unwrap();
    let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    for seen in [&loser, &winner] {
        let mut edit = seen.clone();
        edit.revision += 1;
        edit.pinned = true;
        assert!(bank.commit_expected(edit, Some(seen)).await.is_err());
    }
    assert_eq!(
        std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
        before
    );
}

#[tokio::test]
async fn atomic_preserves_corrupt_audit_bytes_and_recovers_without_half_a_merge() {
    use std::io::Write;
    let (dir, deps) = fixture().await;
    let path = dir.path().join("offline/memories.jsonl");
    // Existing legacy/corrupt audit bytes are never reserialized or discarded.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{torn legacy line")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    items::supersede(&deps, request(None, None)).await.unwrap();
    let after = std::fs::read(&path).unwrap();
    assert!(after.starts_with(&before));
    let store = Store::open(dir.path().into()).unwrap();
    let bank = store.bank("offline").await.unwrap();
    assert!(!bank.get("b").await.unwrap().is_live());
    assert_eq!(bank.get("a").await.unwrap().corroboration, 5);
    // A crash before rename leaves only an unpublished temp file: replay
    // sees neither side of that would-be operation, even if it is valid JSON.
    std::fs::write(
        dir.path().join("offline/.memories.jsonl.tmp"),
        serde_json::to_vec(&record("a")).unwrap(),
    )
    .unwrap();
    let reopened = Store::open(dir.path().into()).unwrap();
    assert_eq!(
        reopened
            .bank("offline")
            .await
            .unwrap()
            .get("a")
            .await
            .unwrap()
            .corroboration,
        5
    );
}

#[tokio::test]
async fn atomic_reload_generation_cannot_absorb_twice_or_publish_a_stale_edit() {
    let (dir, deps) = fixture().await;
    let store = deps.store().await;
    let old_bank = store.bank("offline").await.unwrap();
    let old_loser = old_bank.get("b").await.unwrap();
    let old_winner = old_bank.get("a").await.unwrap();
    store.reload().await.unwrap();
    items::supersede(
        &deps,
        request(Some(json!(old_loser)), Some(json!(old_winner))),
    )
    .await
    .unwrap();
    let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    // This Arc still believes both records are revision zero. The on-disk
    // precondition under the log mutex, not its RAM lock, must reject it.
    assert!(old_bank
        .supersede("b", "a", true, (None, None))
        .await
        .is_err());
    let mut stale = old_winner.clone();
    stale.revision += 1;
    stale.pinned = true;
    assert!(old_bank
        .commit_expected(stale, Some(&old_winner))
        .await
        .is_err());
    assert_eq!(
        std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
        before
    );
}

#[tokio::test]
async fn currentness_pt_en_dedup_real_store_preserves_detail_pinned_rules_history() {
    let (dir, deps) = fixture().await;
    let store = deps.store().await;
    let bank = store.bank("offline").await.unwrap();
    for (id, text) in [
        ("en", "User prefers tea."),
        ("pt", "Usuário prefere chá."),
        ("detail", "Usuário prefere chá à noite."),
    ] {
        let mut m = record(id);
        m.text = text.into();
        m.pinned = id == "en";
        bank.commit(m).await.unwrap();
    }
    let pinned = bank.get("en").await.unwrap();
    let detail = bank.get("detail").await.unwrap();
    let boundary = ActualBoundary {
        deps,
        calls: Mutex::new(vec![]),
        interleave: Mutex::new(None),
    };
    let (report, _) = consolidate::run_bank(&boundary, "offline", false, &mut 10).await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(bank.get("en").await.unwrap(), pinned);
    assert_eq!(bank.get("detail").await.unwrap(), detail);
    let retired = bank.get("pt").await.unwrap();
    assert!(!retired.is_live());
    assert_eq!(retired.superseded_by.as_deref(), Some("en"));
    assert_eq!(retired.source, record("pt").source);
    assert!(report
        .detail_proofs
        .iter()
        .any(|p| p.survivor_source == pinned.text && p.retired_source == retired.text));
    assert_eq!(
        bank.list_rules().unwrap()[0].content,
        "Permanent rule; preserve bytes.\n"
    );
    let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    let (again, _) = consolidate::run_bank(&boundary, "offline", false, &mut 10).await;
    assert_eq!(again.superseded, 0);
    assert_eq!(
        std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
        before
    );
    let reopened = Store::open(dir.path().into()).unwrap();
    assert_eq!(
        reopened
            .bank("offline")
            .await
            .unwrap()
            .get("pt")
            .await
            .unwrap(),
        retired
    );
}
