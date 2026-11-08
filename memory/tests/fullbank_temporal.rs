//! Original raw bank and exact queries through Store, public recall and hook.
//! Cached embeddings are replayed, never generated or inferred from labels.
use futures_util::{SinkExt, StreamExt};
use memory::{
    config::WorkerConfig,
    deps::Deps,
    events::Emitter,
    hooks::{self, PreGenerateInput},
    store::Store,
    types::{fingerprint, now_ms, Confidence, Memory, Provenance},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

struct Boundary {
    deps: Arc<Deps>,
    calls: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Boundary {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

async fn boundary(root: &std::path::Path, cache: Option<Value>) -> Boundary {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let enabled = cache.is_some();
    let model = cache
        .as_ref()
        .and_then(|v| v["model"].as_str())
        .unwrap_or("")
        .to_owned();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let _ = ready.send(());
        while let Some(Ok(frame)) = socket.next().await {
            let Ok(text) = frame.to_text() else { continue };
            let Ok(request) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if request["type"] != "invokefunction" {
                continue;
            }
            let function = request["function_id"].as_str().unwrap();
            let mut reply = json!({"type":"invocationresult","invocation_id":request["invocation_id"],"function_id":function});
            match function {
                "engine::workers::register" => reply["result"] = json!({"success":true}),
                "router::embed" => {
                    let texts = request["data"]["input"].as_array().unwrap();
                    assert_eq!(texts.len(), 1);
                    let text = texts[0].as_str().unwrap();
                    recorded.lock().unwrap().push(text.into());
                    let cached = cache
                        .as_ref()
                        .and_then(|v| v["items"].as_object())
                        .and_then(|items| items.values().find(|v| v["text"] == text));
                    if let Some(vector) = cached {
                        reply["result"] = json!({"model":cache.as_ref().unwrap()["model"],"embeddings":[vector["v"]]});
                    } else {
                        reply["error"] =
                            json!({"code":"cache_miss","message":"No exact cached query"});
                    }
                }
                "session::get" => {
                    reply["result"] =
                        json!({"meta":{"session_id":request["data"]["session_id"],"metadata":{}}})
                }
                _ => {
                    recorded
                        .lock()
                        .unwrap()
                        .push(format!("UNEXPECTED:{function}"));
                    reply["error"] = json!({"code":"unexpected_rpc","message":function});
                }
            }
            if socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    reply.to_string().into(),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let iii = Arc::new(iii_sdk::register_worker(
        &url,
        iii_sdk::InitOptions::default(),
    ));
    wait.await.unwrap();
    Boundary {
        deps: Arc::new(Deps {
            emitter: Arc::new(Emitter::new(iii.clone())),
            iii,
            store: Arc::new(tokio::sync::RwLock::new(Arc::new(
                Store::open(root.into()).unwrap(),
            ))),
            config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
                default_bank: "temporal".into(),
                embeddings_enabled: enabled,
                embedding_model: model,
                extraction_enabled: false,
                recall_budget_tokens: 1200,
                ..Default::default()
            }))),
        }),
        calls,
        server,
    }
}

fn input(session: &str, query: &str) -> PreGenerateInput {
    serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"step":0,
        "metadata":{"memory_bank":"temporal"},
        "recall_context":{"version":1,"session_id":session,"turn_id":format!("turn-{session}"),"source":{"kind":"direct","entry_id":"question"},"query":{"entry_id":"question","text":query}},
        "generate":{"system_prompt":"offline","messages":[{"role":"user","content":query}]}})).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_original_fullbank_temporal_source_contract_all_cached_lanes() {
    let corpus: Value = serde_json::from_str(include_str!("recall_quality/golden.json")).unwrap();
    assert_eq!(corpus["memories"].as_array().unwrap().len(), 76);
    assert_eq!(corpus["queries"].as_array().unwrap().len(), 62);
    assert_eq!(corpus["negatives"].as_array().unwrap().len(), 41);
    let mut failures = Vec::new();
    let mut measured = 0;
    let mut attested_measured = 0;
    let mut stated_measured = 0;
    let mut original_invalid_measured = 0;
    // The old Extracted/sourceNone diagnostic remains a separate negative.
    // Synthetic provenance is NEW producer setup, not recovered corpus history.
    for (provenance_case, confidence, certified) in [
        (
            "invalid:original-Extracted/sourceNone",
            Confidence::Extracted,
            false,
        ),
        (
            "new:synthetic-producer-attested",
            Confidence::Extracted,
            true,
        ),
        ("existing:explicit-Stated", Confidence::Stated, true),
        ("invalid:source-null", Confidence::Extracted, false),
        ("invalid:partial-session-only", Confidence::Extracted, false),
        ("invalid:partial-entry-only", Confidence::Extracted, false),
        ("invalid:blank-entry", Confidence::Extracted, false),
        (
            "invalid:foreign-session-rejected-by-producer",
            Confidence::Extracted,
            false,
        ),
        (
            "invalid:foreign-entry-rejected-by-producer",
            Confidence::Extracted,
            false,
        ),
    ] {
        for (lane, raw) in [
            ("lexical", None),
            (
                "cached-a",
                Some(include_str!("recall_quality/vectors.json")),
            ),
            (
                "cached-b",
                Some(include_str!("recall_quality/vectors-minilm.json")),
            ),
        ] {
            let cache: Option<Value> = raw.map(|s| serde_json::from_str(s).unwrap());
            let dir = tempfile::tempdir().unwrap();
            let stack = boundary(dir.path(), cache.clone()).await;
            let store = stack.deps.store().await;
            let (bank, _) = store.ensure_bank("temporal", None).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let now = now_ms();
            let mut keys = HashMap::new();
            let producer_session = format!(
                "synthetic:fullbank:temporal:{}",
                dir.path().file_name().unwrap().to_string_lossy()
            );
            let mut producer_entries = HashMap::new();
            for fact in corpus["memories"].as_array().unwrap() {
                let key = fact["key"].as_str().unwrap();
                let text = fact["text"].as_str().unwrap();
                let id = fingerprint(text);
                let at = now - fact["age_days"].as_u64().unwrap() * 86_400_000;
                let original = Memory {
                    id: id.clone(),
                    text: text.into(),
                    entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
                    tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
                    confidence,
                    corroboration: fact["corroboration"].as_u64().unwrap_or(0) as u32,
                    pinned: fact["pinned"].as_bool().unwrap_or(false),
                    source: None,
                    created_at: at,
                    updated_at: at,
                    stated_at: Some(at),
                    invalid_at: None,
                    superseded_by: None,
                    revision: 0,
                };
                let entry = format!("synthetic:entry:{id}");
                assert!(producer_entries
                    .insert((producer_session.clone(), entry.clone()), original.clone())
                    .is_none());
                let complete = Provenance {
                    session_id: Some(producer_session.clone()),
                    entry_id: Some(entry),
                    agent: None,
                };
                let mut constructed = original.clone();
                constructed.source = match provenance_case {
                    "new:synthetic-producer-attested" => Some(complete.clone()),
                    "invalid:source-null" => serde_json::from_value(Value::Null).unwrap(),
                    "invalid:partial-session-only" => Some(Provenance {
                        entry_id: None,
                        ..complete.clone()
                    }),
                    "invalid:partial-entry-only" => Some(Provenance {
                        session_id: None,
                        ..complete.clone()
                    }),
                    "invalid:blank-entry" => Some(Provenance {
                        entry_id: Some(" ".into()),
                        ..complete.clone()
                    }),
                    _ => None,
                };
                if provenance_case.starts_with("invalid:foreign-") {
                    let mut foreign = complete.clone();
                    if provenance_case == "invalid:foreign-session-rejected-by-producer" {
                        foreign.session_id = Some(format!("foreign:{producer_session}"));
                    } else {
                        foreign.entry_id = Some(format!("foreign:{}", foreign.entry_id.unwrap()));
                    }
                    let reference = (foreign.session_id.unwrap(), foreign.entry_id.unwrap());
                    assert!(
                        !producer_entries.contains_key(&reference),
                        "foreign reference must not certify this producer"
                    );
                    // Production recall validates non-empty shape, not ownership.
                    // This producer rejects unresolvable foreign references,
                    // leaving sourceNone; it does NOT claim product resolution.
                    assert!(constructed.source.is_none());
                }
                if provenance_case == "new:synthetic-producer-attested" {
                    let source = constructed.source.as_ref().unwrap();
                    let resolved = &producer_entries[&(
                        source.session_id.clone().unwrap(),
                        source.entry_id.clone().unwrap(),
                    )];
                    assert_eq!(resolved, &original);
                    assert_eq!(constructed.confidence, Confidence::Extracted);
                    // Non-empty foreign references satisfy the production shape
                    // check, NOT this producer's ownership contract. Never put
                    // them into the bank as certified synthetic provenance.
                    assert!(!producer_entries.contains_key(&(
                        format!("foreign:{producer_session}"),
                        source.entry_id.clone().unwrap()
                    )));
                    assert!(!producer_entries.contains_key(&(
                        producer_session.clone(),
                        format!("foreign:{}", source.entry_id.as_ref().unwrap())
                    )));
                }
                let mut without_source = constructed.clone();
                without_source.source = None;
                assert_eq!(without_source, original, "all non-source fields: {key}");
                bank.commit(constructed.clone()).await.unwrap();
                assert_eq!(bank.get(&id).await.as_ref(), Some(&constructed));
                if let Some(cache) = &cache {
                    assert_eq!(cache["items"][key]["text"], text);
                    let vector: Vec<f32> =
                        serde_json::from_value(cache["items"][key]["v"].clone()).unwrap();
                    assert_eq!(vector.len(), cache["dims"].as_u64().unwrap() as usize);
                    bank.set_vector(&id, cache["model"].as_str().unwrap(), text, vector)
                        .await
                        .unwrap();
                }
                keys.insert(id, key.to_owned());
            }
            if provenance_case == "new:synthetic-producer-attested" {
                for q in corpus["queries"].as_array().unwrap() {
                    for old_key in q["stale"].as_array().into_iter().flatten() {
                        for new_key in q["expect"].as_array().unwrap() {
                            let old_id = keys
                                .iter()
                                .find(|(_, key)| key.as_str() == old_key.as_str().unwrap())
                                .unwrap()
                                .0;
                            let new_id = keys
                                .iter()
                                .find(|(_, key)| key.as_str() == new_key.as_str().unwrap())
                                .unwrap()
                                .0;
                            let old = bank.get(old_id).await.unwrap();
                            let new = bank.get(new_id).await.unwrap();
                            if !old.revalidated_by(&new) {
                                continue;
                            }
                            let mut pinned = old.clone();
                            pinned.pinned = true;
                            assert!(!pinned.revalidated_by(&new));
                            let mut inferred = new.clone();
                            inferred.confidence = Confidence::Inferred;
                            assert!(!old.revalidated_by(&inferred));
                            let mut unknown = new.clone();
                            unknown.source = None;
                            assert!(!old.revalidated_by(&unknown));
                            let mut equal_date = new.clone();
                            equal_date.stated_at = old.stated_at;
                            equal_date.updated_at = old.updated_at;
                            assert!(!old.revalidated_by(&equal_date));
                        }
                    }
                }
            }
            let path = dir.path().join("temporal/memories.jsonl");
            let before = std::fs::read(&path).unwrap();
            for query_id in [
                "q-upd-ci-current",
                "q-upd-ci-pipeline",
                "q-upd-run-day",
                "q-upd-editor",
            ] {
                let q = corpus["queries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|q| q["id"] == query_id)
                    .unwrap();
                let query = q["text"].as_str().unwrap();
                let required: HashSet<String> =
                    serde_json::from_value(q["expect"].clone()).unwrap();
                let forbidden: HashSet<String> =
                    serde_json::from_value(q["stale"].clone()).unwrap();
                let qv: Option<Vec<f32>> = cache.as_ref().map(|cache| {
                    assert_eq!(cache["items"][query_id]["text"], query);
                    serde_json::from_value(cache["items"][query_id]["v"].clone()).unwrap()
                });
                let model = cache
                    .as_ref()
                    .and_then(|cache| cache["model"].as_str())
                    .unwrap_or("");
                let (ranked, evidence) = bank
                    .recall_evidence(query, qv.as_deref(), model, usize::MAX, 30, false, 0.0)
                    .await;
                let raw_ranked: Vec<_> = ranked.iter().map(|(m, _)| keys[&m.id].clone()).collect();
                let reasons = hooks::admission_reasons(&ranked, &evidence);
                let selected = hooks::select(ranked, &evidence, &HashSet::new(), 6, query);
                let selected: HashSet<_> =
                    selected.iter().map(|(m, _)| keys[&m.id].clone()).collect();
                let (limited, _) = bank
                    .recall_evidence(query, qv.as_deref(), model, 1, 30, false, 0.0)
                    .await;
                let public = memory::functions::recall::recall(
                    &stack.deps,
                    serde_json::from_value(json!({"bank":"temporal","query":query,"limit":100}))
                        .unwrap(),
                )
                .await
                .unwrap();
                let public_keys: HashSet<_> = public
                    .memories
                    .iter()
                    .map(|m| keys[&m.memory.id].clone())
                    .collect();
                if cache.is_some() {
                    assert_eq!(public.retrieval, "bm25-entity-semantic");
                }
                let response = hooks::pre_generate(&stack.deps, input(query_id, query))
                    .await
                    .unwrap();
                let annotations = response.annotations.as_ref().unwrap();
                let ids: Vec<String> = serde_json::from_value(
                    annotations.get("memory_ids").cloned().unwrap_or(json!([])),
                )
                .unwrap();
                let injected: HashSet<_> = ids.iter().map(|id| keys[id].clone()).collect();
                let supported: Vec<_> = reasons
                    .iter()
                    .filter(|(_, r)| r.supported())
                    .map(|(id, _)| keys[id].clone())
                    .collect();
                // Unattested extraction is a source negative, not an accepted
                // factual answer: original required/forbidden sets below stay
                // unchanged and factual counters still report the stale leak.
                let expected: HashSet<_> = if certified {
                    required.clone()
                } else {
                    required.union(&forbidden).cloned().collect()
                };
                let (history, _) = bank
                    .recall_evidence(query, qv.as_deref(), model, usize::MAX, 30, true, 0.0)
                    .await;
                for stale in &forbidden {
                    let id = keys.iter().find(|(_, key)| *key == stale).unwrap().0;
                    assert!(
                        history.iter().any(|(m, _)| &m.id == id),
                        "history {query_id}/{lane}"
                    );
                }
                if selected != expected || injected != expected {
                    failures.push(format!("{provenance_case}/{confidence:?}/{lane}/{query_id}: expected {expected:?}; select {selected:?}; hook {injected:?}"));
                }
                if certified
                    && (!required.iter().all(|id| raw_ranked.contains(id))
                        || raw_ranked.iter().any(|id| forbidden.contains(id))
                        || !required.is_subset(&public_keys)
                        || !forbidden.is_disjoint(&public_keys)
                        || !limited
                            .iter()
                            .all(|(m, _)| !forbidden.contains(&keys[&m.id])))
                {
                    failures.push(format!(
                        "public/prelimit {provenance_case}/{lane}/{query_id}: {public_keys:?}"
                    ));
                }
                // Factual counters are separate from any static policy verdict.
                println!(
                    "{}",
                    json!({"query":query_id,"lane":lane,"confidence":confidence,"provenance_case":provenance_case,"certified":certified,"bank_size":keys.len(),"before_select":raw_ranked,"supported":supported,"selected":selected,"hook":injected,"required":required,"forbidden":forbidden,"required_recalled":selected.intersection(&required).count(),"forbidden_selected":selected.intersection(&forbidden).count(),"noise":selected.difference(&required).count(),"precision":selected.intersection(&required).count() as f64 / selected.len().max(1) as f64,"required_recall":selected.intersection(&required).count() as f64 / required.len() as f64})
                );
                if provenance_case == "new:synthetic-producer-attested" {
                    attested_measured += 1;
                } else if provenance_case == "existing:explicit-Stated" {
                    stated_measured += 1;
                } else if provenance_case == "invalid:original-Extracted/sourceNone" {
                    original_invalid_measured += 1;
                }
                measured += 1;
            }
            assert_eq!(std::fs::read(path).unwrap(), before);
            assert_eq!(
                bank.list_rules().unwrap()[0].content,
                "Preserve the permanent safety rule."
            );
            let calls = stack.calls.lock().unwrap();
            assert!(
                !calls.iter().any(|call| call.starts_with("UNEXPECTED:")),
                "{calls:?}"
            );
            assert_eq!(calls.len(), if cache.is_some() { 8 } else { 0 });
        }
    }
    assert_eq!(attested_measured, 12);
    assert_eq!(stated_measured, 12);
    assert_eq!(original_invalid_measured, 12);
    assert_eq!(measured, 108);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
