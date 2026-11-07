// Real pre_generate + SDK transport + stored cached vectors. The only
// embedding service is this ephemeral loopback mock; cache misses error, never
// synthesize a vector. This measures integration, not a newly run embedder.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::Mutex;

pub(super) struct Boundary {
    pub(super) deps: Arc<Deps>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) sessions: Arc<Mutex<HashMap<String, Value>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Boundary {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

pub(super) async fn boundary(dir: &std::path::Path, cache: Value) -> Boundary {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let sessions = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
    let session_metadata = sessions.clone();
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
            if function == "engine::workers::register" {
                // SDK connection handshake, not an embedding or an arbitrary
                // RPC exemption. Every other unknown target still fails.
                reply["result"] = json!({"success":true});
            } else if function == "router::embed" {
                let input = request["data"]["input"].as_array().unwrap();
                assert_eq!(input.len(), 1);
                let text = input[0].as_str().unwrap();
                recorded.lock().unwrap().push(text.into());
                let cached = cache["items"]
                    .as_object()
                    .unwrap()
                    .values()
                    .find(|v| v["text"] == text);
                if let Some(v) = cached {
                    reply["result"] = json!({"model":cache["model"],"embeddings":[v["v"]]});
                } else {
                    reply["error"] = json!({"code":"cache_miss","message":"No vector exists for this exact query"});
                }
            } else if function == "session::get" {
                reply["result"] = json!({"meta":{"session_id":request["data"]["session_id"],"metadata":session_metadata.lock().unwrap().get(request["data"]["session_id"].as_str().unwrap()).cloned().unwrap_or_else(||json!({}))}});
            } else {
                recorded
                    .lock()
                    .unwrap()
                    .push(format!("UNEXPECTED:{function}"));
                reply["error"] = json!({"code":"unexpected_rpc","message":function});
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
    let deps = Arc::new(Deps {
        emitter: Arc::new(crate::events::Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(
            crate::store::Store::open(dir.to_path_buf()).unwrap(),
        ))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
            embeddings_enabled: true,
            extraction_enabled: false,
            default_bank: "cached-main".into(),
            ..WorkerConfig::default()
        }))),
    });
    Boundary {
        deps,
        calls,
        sessions,
        server,
    }
}

fn input(session: &str, query: &str, kind: &str) -> PreGenerateInput {
    serde_json::from_value(json!({
        "session_id":session,"turn_id":format!("t-{session}"),"step":0,
        "metadata":{"memory_bank":"cached-main"},
        "recall_context":{"version":1,"session_id":session,"turn_id":format!("t-{session}"),
            "source":{"kind":kind,"entry_id":"e1"},"query":{"entry_id":"e1","text":query}},
        "generate":{"system_prompt":"offline","messages":[{"role":"user","content":query}]}
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hybrid_policy_real_cache_pairs_abstain_without_support_and_keep_explicit_answers() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    for raw in [
        include_str!("../recall_quality/vectors.json"),
        include_str!("../recall_quality/vectors-minilm.json"),
    ] {
        let cache: Value = serde_json::from_str(raw).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let stack = boundary(dir.path(), cache.clone()).await;
        let store = stack.deps.store().await;
        let (bank, _) = store.ensure_bank("cached-main", None).await.unwrap();
        bank.set_rule("standing", "Keep independent support rule.")
            .unwrap();
        for fact in corpus["memories"].as_array().unwrap() {
            let id = fact["key"].as_str().unwrap();
            let text = fact["text"].as_str().unwrap();
            let cached = &cache["items"][id];
            assert_eq!(cached["text"], text);
            let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
            bank.commit(Memory {
                id: id.into(),
                text: text.into(),
                tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
                entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
                pinned: fact["pinned"].as_bool().unwrap_or(false),
                confidence: crate::types::Confidence::Stated,
                corroboration: 0,
                source: None,
                created_at: at,
                updated_at: at,
                stated_at: Some(at),
                invalid_at: None,
                superseded_by: None,
                revision: 0,
            })
            .await
            .unwrap();
            let v: Vec<f32> = serde_json::from_value(cached["v"].clone()).unwrap();
            assert_eq!(v.len(), cache["dims"].as_u64().unwrap() as usize);
            bank.set_vector(id, cache["model"].as_str().unwrap(), text, v)
                .await
                .unwrap();
        }
        let mut failures = Vec::new();
        let mut executed = 0;
        // Expected ids are independently labelled, not calculated by admission_reasons.
        for (n, (query_id, expected)) in [
            ("q-pt-life-par-mana", None),
            ("q-evt-next-week", None),
            ("q-en-work-release-days", None),
            ("q-en-work-ledgerly-db", Some("en-work-ledgerly-postgres")),
            ("q-pt-life-lex-escalada", Some("pt-life-escalada-boulder")),
        ]
        .into_iter()
        .enumerate()
        {
            let query = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap()["text"]
                .as_str()
                .unwrap();
            assert_eq!(
                cache["items"][query_id]["text"], query,
                "exact genuine query cache required"
            );
            let qv: Vec<f32> =
                serde_json::from_value(cache["items"][query_id]["v"].clone()).unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(
                    query,
                    Some(&qv),
                    cache["model"].as_str().unwrap(),
                    usize::MAX,
                    30,
                    false,
                    0.0,
                )
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            let response = serde_json::to_value(
                pre_generate(
                    &stack.deps,
                    input(&format!("hybrid-policy-{n}"), query, "direct"),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            let selected = response["annotations"]
                .get("memory_ids")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let expected = expected.map_or_else(|| json!([]), |id| json!([id]));
            println!(
                "{}",
                json!({"diagnostic":"hybrid-policy-real-cache","query_id":query_id,
                "model":cache["model"],"selected":selected,"expected":expected,
                "admission_reasons":reasons,"candidate_evidence":ranked.iter().take(8).map(|(m,_)|{
                    let e=&evidence[&m.id];json!({"id":m.id,"matched":e.matched,"cosine":e.cosine,"z":e.z})
                }).collect::<Vec<_>>(),"embedding_quality_measured":true,"provider_calls":0})
            );
            executed += 1;
            if selected != expected {
                failures.push(format!("{query_id}: {response}"));
            }
            if !response["mutations"]["system_prompt"]
                .as_str()
                .unwrap_or("")
                .contains("Keep independent support rule.")
            {
                failures.push(format!("rule lost: {query_id}"));
            }
        }
        assert_eq!(executed, 5);
        assert!(!stack
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.starts_with("UNEXPECTED:")));
        assert_eq!(stack.calls.lock().unwrap().len(), 5);
        assert!(
            failures.is_empty(),
            "precision-first cache pairs: {failures:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_boundary_real_hook_retrieval_and_notification_later_step_fallback() {
    for raw in [
        include_str!("../recall_quality/vectors.json"),
        include_str!("../recall_quality/vectors-minilm.json"),
    ] {
        let cache: Value = serde_json::from_str(raw).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let stack = boundary(dir.path(), cache.clone()).await;
        let store = stack.deps.store().await;
        let (bank, _) = store.ensure_bank("cached-main", None).await.unwrap();
        bank.set_rule("standing", "Keep cached integration rule.")
            .unwrap();
        let corpus: Value =
            serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
        for f in corpus["memories"].as_array().unwrap() {
            let id = f["key"].as_str().unwrap();
            let c = &cache["items"][id];
            assert_eq!(c["text"], f["text"]);
            let m = Memory {
                id: id.into(),
                text: f["text"].as_str().unwrap().into(),
                entities: serde_json::from_value(f["entities"].clone()).unwrap_or_default(),
                tags: Vec::new(),
                confidence: crate::types::Confidence::Stated,
                corroboration: 0,
                pinned: f["pinned"].as_bool().unwrap_or(false),
                source: None,
                created_at: now_ms(),
                updated_at: now_ms(),
                stated_at: None,
                invalid_at: None,
                superseded_by: None,
                revision: 0,
            };
            bank.commit(m.clone()).await.unwrap();
            let v: Vec<f32> = serde_json::from_value(c["v"].clone()).unwrap();
            assert_eq!(v.len(), cache["dims"].as_u64().unwrap() as usize);
            assert!(v.iter().all(|x| x.is_finite()));
            bank.set_vector(id, cache["model"].as_str().unwrap(), &m.text, v)
                .await
                .unwrap();
        }
        let query = cache["items"]["q-en-work-ledgerly-db"]["text"]
            .as_str()
            .unwrap();
        let out = serde_json::to_value(
            pre_generate(&stack.deps, input("cached-query", query, "direct"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            out["annotations"]["memory_retrieval"],
            "bm25-entity-semantic"
        );
        assert!(
            out["annotations"]["memory_ids"]
                .as_array()
                .unwrap()
                .contains(&json!("en-work-ledgerly-postgres")),
            "{out}"
        );
        assert_eq!(stack.calls.lock().unwrap().as_slice(), [query]);
        let notice = pre_generate(&stack.deps, input("cached-notice", query, "notification"))
            .await
            .unwrap();
        assert!(notice.annotations.unwrap().get("memory_ids").is_none());
        let mut later = input("cached-query", query, "direct");
        later.step = 1;
        let out = pre_generate(&stack.deps, later).await.unwrap();
        assert!(out.annotations.unwrap().get("memory_ids").is_none());
        assert_eq!(
            stack.calls.lock().unwrap().len(),
            1,
            "notification/later step must not embed"
        );
        // New text has no cached vector. The real client errors and production
        // falls back lexically; this is NOT a quality measurement of an embedder.
        let novel = "Recall Ledgerly PostgreSQL version from stored facts.";
        let out = serde_json::to_value(
            pre_generate(&stack.deps, input("cache-miss", novel, "direct"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            out["annotations"]["memory_ids"]
                .as_array()
                .unwrap()
                .contains(&json!("en-work-ledgerly-postgres")),
            "{out}"
        );
        assert!(out["annotations"].get("memory_retrieval").is_none());
        assert_eq!(stack.calls.lock().unwrap().as_slice(), [query, novel]);
        // The same exact cached query traverses a small bank and a derived
        // bank with a user layer through the real resolver and injection.
        let cached = &cache["items"]["en-work-ledgerly-postgres"];
        let mut m = bank.get("en-work-ledgerly-postgres").await.unwrap();
        m.entities.push("user".into());
        bank.commit(m.clone()).await.unwrap();
        let (small, _) = store.ensure_bank("cached-small", None).await.unwrap();
        small.commit(m.clone()).await.unwrap();
        small
            .set_vector(
                &m.id,
                cache["model"].as_str().unwrap(),
                &m.text,
                serde_json::from_value(cached["v"].clone()).unwrap(),
            )
            .await
            .unwrap();
        let mut small_input = input("small-cached", query, "direct");
        small_input.metadata = Some(json!({"memory_bank":"cached-small"}));
        let out =
            serde_json::to_value(pre_generate(&stack.deps, small_input).await.unwrap()).unwrap();
        assert!(
            out["annotations"]["memory_ids"]
                .as_array()
                .unwrap()
                .contains(&json!("en-work-ledgerly-postgres")),
            "{out}"
        );
        let corpus: Value =
            serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
        let cached_negative = corpus["negatives"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|q| q["kind"] == "chatter" || q["kind"] == "greeting")
            .find_map(|q| {
                let id = q["id"].as_str()?;
                let text = q["text"].as_str()?;
                (cache["items"][id]["text"] == text).then_some((id, text))
            });
        let (negative_id, negative) = cached_negative.unwrap_or_else(|| {
            let q = corpus["negatives"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["kind"] == "greeting")
                .unwrap();
            (q["id"].as_str().unwrap(), q["text"].as_str().unwrap())
        });
        assert!(negative_id.starts_with("neg-"));
        println!(
            "{}",
            json!({"cache_negative":negative_id,"text":negative,"model":cache["model"],"embedding_negative_measured":cached_negative.is_some(),"fallback_negative_measured":true,"coverage_complete":cached_negative.is_some()})
        );
        let calls_before_negative = stack.calls.lock().unwrap().len();
        let mixed_negative = serde_json::to_value(
            pre_generate(&stack.deps, input("mixed-negative", negative, "direct"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            mixed_negative["annotations"].get("memory_ids").is_none(),
            "{mixed_negative}"
        );
        assert_eq!(stack.calls.lock().unwrap().len(), calls_before_negative + 1);
        assert_eq!(stack.calls.lock().unwrap().last().unwrap(), negative);
        let mut small_input = input("small-negative", negative, "direct");
        small_input.metadata = Some(json!({"memory_bank":"cached-small"}));
        let out =
            serde_json::to_value(pre_generate(&stack.deps, small_input).await.unwrap()).unwrap();
        assert!(out["annotations"].get("memory_ids").is_none(), "{out}");
        let mut layer_input = input("layer-cached", query, "direct");
        layer_input.metadata = Some(json!({"fs_scope":{"root":dir.path()}}));
        let out =
            serde_json::to_value(pre_generate(&stack.deps, layer_input).await.unwrap()).unwrap();
        assert!(
            out["annotations"]["memory_ids"]
                .as_array()
                .unwrap()
                .contains(&json!("en-work-ledgerly-postgres")),
            "{out}"
        );
        assert!(
            out["annotations"]["memory_banks"]
                .as_array()
                .unwrap()
                .contains(&json!("cached-main")),
            "{out}"
        );
        assert!(out["mutations"]["system_prompt"]
            .as_str()
            .unwrap()
            .contains("Keep cached integration rule."));
        assert!(!stack
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("UNEXPECTED:")));
    }
}
