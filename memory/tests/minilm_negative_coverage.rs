//! Additive MiniLM negative proof, not a replacement for either frozen cache.
//! The 41 exact negative queries were exported once by the offline local driver.
//! Runtime uses only those vectors and the original authentic memory vectors.
//! An ephemeral SDK transport serves exact texts; a miss is a hard test failure,
//! never semantic evidence. No engine, provider, extraction or live bank is used.
use futures_util::{SinkExt, StreamExt};
use memory::config::WorkerConfig;
use memory::deps::Deps;
use memory::events::Emitter;
use memory::hooks::{admission_reasons, pre_generate, select, PreGenerateInput};
use memory::store::Store;
use memory::types::{now_ms, Confidence, Memory};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

const MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2@c9745ed1d9f2";
const RULE: &str = "Preserve the permanent synthetic safety rule.";
const INPUTS: &str = include_str!("injection_replay/MiniLMnegativeproof_inputs.json");
const NEGATIVES: &str = include_str!("injection_replay/MiniLMnegativeproof_vectors.json");
const MEMORIES: &str = include_str!("recall_quality/vectors-minilm.json");
const GOLDEN: &str = include_str!("recall_quality/golden.json");

#[derive(Clone, Deserialize)]
struct Embedded {
    text: String,
    v: Vec<f32>,
}

#[derive(Clone, Deserialize)]
struct Cache {
    model: String,
    dims: usize,
    items: BTreeMap<String, Embedded>,
}

impl Cache {
    fn parse(raw: &str, l2_tolerance: f32) -> Self {
        let cache: Self = serde_json::from_str(raw).unwrap();
        assert_eq!(cache.model, MODEL);
        assert_eq!(cache.dims, 384);
        for item in cache.items.values() {
            assert_eq!(item.v.len(), 384);
            assert!(item.v.iter().all(|n| n.is_finite()));
            assert!((item.v.iter().map(|n| n * n).sum::<f32>() - 1.0).abs() < l2_tolerance);
        }
        cache
    }

    fn exact(&self, text: &str) -> &Embedded {
        let matches: Vec<_> = self.items.values().filter(|v| v.text == text).collect();
        assert_eq!(matches.len(), 1, "exact unique authentic text: {text}");
        matches[0]
    }
}

// No writes to existing fixtures or shared test helpers. The productive SDK,
// query client, Store, selection and hook are used unmodified through public API.
struct Boundary {
    deps: Arc<Deps>,
    calls: Arc<Mutex<Vec<String>>>,
    misses: Arc<Mutex<Vec<String>>>,
    unexpected: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Boundary {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

async fn boundary(root: &std::path::Path, cache: Cache) -> Boundary {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let misses = Arc::new(Mutex::new(Vec::new()));
    let unexpected = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let absent = misses.clone();
    let unknown = unexpected.clone();
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
            let mut reply = json!({"type":"invocationresult",
                "invocation_id":request["invocation_id"],"function_id":function});
            match function {
                "engine::workers::register" => reply["result"] = json!({"success":true}),
                "router::embed" => {
                    let input = request["data"]["input"].as_array().unwrap();
                    assert_eq!(input.len(), 1, "one query, never candidate backfill");
                    assert_eq!(request["data"]["model"], MODEL);
                    let text = input[0].as_str().unwrap();
                    recorded.lock().unwrap().push(text.to_owned());
                    let found: Vec<_> = cache.items.values().filter(|v| v.text == text).collect();
                    if found.len() == 1 {
                        reply["result"] = json!({"model":MODEL,"embeddings":[found[0].v]});
                    } else {
                        absent.lock().unwrap().push(text.to_owned());
                        reply["error"] =
                            json!({"code":"cache_miss","message":"exact text absent or ambiguous"});
                    }
                }
                "session::get" => {
                    reply["result"] =
                        json!({"meta":{"session_id":request["data"]["session_id"],"metadata":{}}});
                }
                _ => {
                    unknown.lock().unwrap().push(function.to_owned());
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
    let deps = Arc::new(Deps {
        emitter: Arc::new(Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(
            Store::open(root.to_path_buf()).unwrap(),
        ))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
            embeddings_enabled: true,
            embedding_model: MODEL.into(),
            extraction_enabled: false,
            default_bank: "negative-proof".into(),
            ..WorkerConfig::default()
        }))),
    });
    Boundary {
        deps,
        calls,
        misses,
        unexpected,
        server,
    }
}

fn input(session: &str, query: &str, kind: &str, bank: &str) -> PreGenerateInput {
    serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"step":0,
        "metadata":{"memory_bank":bank},
        "recall_context":{"version":1,"session_id":session,"turn_id":format!("turn-{session}"),
            "source":{"kind":kind,"entry_id":"current"},"query":{"entry_id":"current","text":query}},
        "generate":{"system_prompt":"offline synthetic proof","messages":[{"role":"user","content":query}]}
    })).unwrap()
}

fn record(id: &str, text: &str) -> Memory {
    Memory {
        id: id.into(),
        text: text.into(),
        entities: Vec::new(),
        tags: Vec::new(),
        confidence: Confidence::Stated,
        corroboration: 0,
        pinned: false,
        source: None,
        created_at: now_ms(),
        updated_at: now_ms(),
        stated_at: None,
        invalid_at: None,
        superseded_by: None,
        revision: 0,
    }
}

fn ids(response: &Value) -> Value {
    response["annotations"]
        .get("memory_ids")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn rules(response: &Value) {
    assert!(
        response["mutations"]["system_prompt"]
            .as_str()
            .unwrap_or("")
            .contains(RULE),
        "{response}"
    );
    assert_eq!(response["annotations"]["memory_rules"], 1);
}

fn no_facts_rendered(response: &Value, facts: &[Memory]) {
    let rendered = response["mutations"]["system_prompt"]
        .as_str()
        .unwrap_or("")
        .to_owned()
        + &response["mutations"]["append_messages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|message| memory::extract::content_text(message.get("content")))
            .collect::<Vec<_>>()
            .join("\n");
    for fact in facts {
        assert!(
            !rendered.contains(&fact.text),
            "unrelated text leaked: {}",
            fact.id
        );
    }
}

fn clean(stack: &Boundary) {
    assert!(
        stack.misses.lock().unwrap().is_empty(),
        "cache miss is not semantic coverage"
    );
    assert!(
        stack.unexpected.lock().unwrap().is_empty(),
        "unexpected RPC"
    );
    assert!(
        !stack.server.is_finished(),
        "SDK transport unexpectedly stopped"
    );
}

// Deliberately no custom test-only scoring or evaluator, no snapshot updates.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_41_original_negatives_use_authentic_query_and_memory_vectors_and_abstain() {
    let original: Value = serde_json::from_str(GOLDEN).unwrap();
    let inputs: Value = serde_json::from_str(INPUTS).unwrap();
    let generated = Cache::parse(NEGATIVES, 0.00001);
    // Frozen original vectors have four-decimal quantization (squared norm
    // 0.99984802..1.00016395); preserve bytes, never normalize/substitute.
    // Newly inferred and prior38 full-precision vectors use 1e-5 instead.
    let memories = Cache::parse(MEMORIES, 0.0002);
    let provenance: Value = serde_json::from_str(NEGATIVES).unwrap();
    assert_eq!(
        provenance["provenance"]["input_sha256"],
        "d4a83910d6eff75705077da8da84ca2b501edfda70b11d9427a5634c1373106a"
    );
    assert_eq!(
        provenance["provenance"]["model_sha256"],
        "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452"
    );
    assert_eq!(
        provenance["provenance"]["tokenizer_sha256"],
        "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037"
    );
    assert_eq!(
        provenance["provenance"]["revision"],
        "c9745ed1d9f207416be6d2e6f8de32d1f16199bf"
    );
    assert_eq!(provenance["provenance"]["normalization"], "L2 f32");
    assert_eq!(provenance["provenance"]["pooling"], "mean");
    assert_eq!(provenance["provenance"]["provider_calls"], 0);
    assert_eq!(provenance["provenance"]["synthetic_only"], true);
    assert_eq!(generated.items.len(), 41);
    let negatives = original["negatives"].as_array().unwrap();
    assert_eq!(negatives.len(), 41);
    assert_eq!(inputs.as_array().unwrap().len(), 41);
    let mut unique_ids = HashSet::new();
    for (negative, entry) in negatives.iter().zip(inputs.as_array().unwrap()) {
        assert_eq!(negative["id"], entry["id"]);
        assert_eq!(negative["text"], entry["query"]);
        assert_eq!(entry["candidates"], json!([]), "no extra inference inputs");
        assert!(unique_ids.insert(negative["id"].as_str().unwrap()));
        generated.exact(negative["text"].as_str().unwrap());
    }
    let dir = tempfile::tempdir().unwrap();
    let stack = boundary(dir.path(), generated.clone()).await;
    let store = stack.deps.store().await;
    let (bank, _) = store.ensure_bank("negative-proof", None).await.unwrap();
    bank.set_rule("standing", RULE).unwrap();
    let mut facts = Vec::new();
    for fact in original["memories"].as_array().unwrap() {
        let id = fact["key"].as_str().unwrap();
        let text = fact["text"].as_str().unwrap();
        let cached = &memories.items[id];
        assert_eq!(cached.text, text);
        let mut memory = record(id, text);
        memory.entities = serde_json::from_value(fact["entities"].clone()).unwrap_or_default();
        memory.tags = serde_json::from_value(fact["tags"].clone()).unwrap_or_default();
        memory.pinned = fact["pinned"].as_bool().unwrap_or(false);
        memory.corroboration = fact["corroboration"].as_u64().unwrap_or(0) as u32;
        let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
        memory.created_at = at;
        memory.updated_at = at;
        memory.stated_at = Some(at);
        bank.commit(memory.clone()).await.unwrap();
        assert!(bank
            .set_vector(id, MODEL, text, cached.v.clone())
            .await
            .unwrap());
        facts.push(memory);
    }
    assert_eq!(bank.vector_count().await, facts.len());
    assert!(facts.iter().any(|m| m.pinned), "pin controls must exist");
    let mut failures = Vec::new();
    let (mut executed, mut candidate_cases, mut cosine_cases, mut empty_cases) = (0, 0, 0, 0);
    for negative in negatives {
        let id = negative["id"].as_str().unwrap();
        let text = negative["text"].as_str().unwrap();
        let query = generated.exact(text);
        let (ranked, evidence) = bank
            .recall_evidence(text, Some(&query.v), MODEL, usize::MAX, 30, false, 0.0)
            .await;
        let reasons = admission_reasons(&ranked, &evidence);
        let supported: Vec<_> = ranked
            .iter()
            .filter(|(m, _)| reasons.get(&m.id).is_some_and(|r| r.supported()))
            .map(|(m, _)| m.id.clone())
            .collect();
        let selected = select(ranked.clone(), &evidence, &HashSet::new(), 6, text);
        for (m, _) in &ranked {
            assert!(
                evidence[&m.id].vector,
                "candidate must have authentic compatible vector: {}",
                m.id
            );
            assert_eq!(memories.items[&m.id].text, m.text);
        }
        let cosine_hits = ranked
            .iter()
            .filter(|(m, _)| evidence[&m.id].cosine.is_some_and(|c| c.is_finite()))
            .count();
        candidate_cases += usize::from(!ranked.is_empty());
        cosine_cases += usize::from(cosine_hits > 0);
        empty_cases += usize::from(ranked.is_empty());
        let before = stack.calls.lock().unwrap().len();
        let response = serde_json::to_value(
            pre_generate(&stack.deps, input(id, text, "direct", "negative-proof"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stack.calls.lock().unwrap().len(), before + 1);
        assert_eq!(stack.calls.lock().unwrap().last().unwrap(), text);
        assert_eq!(
            response["annotations"]["memory_retrieval"], "bm25-entity-semantic",
            "no lexical fallback: {id}"
        );
        rules(&response);
        if !supported.is_empty() || !selected.is_empty() || ids(&response) != json!([]) {
            failures.push(format!(
                "{id}: preselect={supported:?}; selected={:?}; hook={}",
                selected.iter().map(|(m, _)| &m.id).collect::<Vec<_>>(),
                ids(&response)
            ));
        }
        if ids(&response) == json!([]) {
            no_facts_rendered(&response, &facts);
        }
        println!(
            "{}",
            json!({"diagnostic":"minilm-negative-proof","id":id,"kind":negative["kind"],"split":negative["split"],
            "model":MODEL,"query_vector":true,"candidate_vectors":ranked.len(),"cosine_candidates":cosine_hits,
            "zero_candidate":ranked.is_empty(),"lexical_fallback":false,"preselect_supported":supported,
            "selected":ids(&response),"noise":ids(&response).as_array().unwrap().len(),"provider_calls":0,
            "evidence":ranked.iter().map(|(m,_)| {let e=&evidence[&m.id];json!({"id":m.id,"vector":e.vector,"cosine":e.cosine,"z":e.z,"reason":reasons.get(&m.id)})}).collect::<Vec<_>>() })
        );
        executed += 1;
    }
    for fact in &facts {
        assert_eq!(
            bank.get(&fact.id).await.unwrap(),
            *fact,
            "recall must not mutate pinned or unpinned facts"
        );
    }
    clean(&stack);
    assert_eq!(executed, 41);
    assert_eq!(stack.calls.lock().unwrap().len(), 41);
    assert_eq!(candidate_cases + empty_cases, 41);
    assert!(
        cosine_cases > 0,
        "nonzero actual semantic candidate coverage required"
    );
    println!(
        "{}",
        json!({"diagnostic":"minilm-negative-total","executed":executed,"candidate_cases":candidate_cases,
        "cosine_positive_cases":cosine_cases,"zero_candidate_cases":empty_cases,"failures":failures,
        "openai256_newpositive":"NOT RUN","global_quality_claim":false})
    );
    assert!(
        failures.is_empty(),
        "negative abstention/noise/unsupported must all be zero: {failures:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_neutral_minilm_pairs_keep_positive_and_counterexample_roles_separate() {
    // Reuse the prior 38-input export; no regeneration or additional inference.
    let cases: Value = serde_json::from_str(include_str!(
        "injection_replay/directive_minilm_inputs.json"
    ))
    .unwrap();
    let cache = Cache::parse(
        include_str!("injection_replay/directive_minilm_vectors.json"),
        0.00001,
    );
    let (mut executed, mut negatives) = (0, 0);
    let mut failures = Vec::new();
    for family in cases.as_array().unwrap().iter().filter(|f| {
        matches!(
            f["id"].as_str(),
            Some("pt-neutral" | "en-neutral-coordination")
        )
    }) {
        let query = family["query"].as_str().unwrap();
        let qv = &cache.exact(query).v;
        let candidates = family["candidates"].as_array().unwrap();
        for index in 0..=candidates.len() {
            let pool = if index == candidates.len() {
                &candidates[..]
            } else {
                &candidates[index..index + 1]
            };
            let expected: Vec<_> = pool
                .iter()
                .filter(|c| c["id"] == "positive")
                .map(|c| c["id"].as_str().unwrap())
                .collect();
            let dir = tempfile::tempdir().unwrap();
            let stack = boundary(dir.path(), cache.clone()).await;
            let store = stack.deps.store().await;
            let (bank, _) = store.ensure_bank("negative-proof", None).await.unwrap();
            bank.set_rule("standing", RULE).unwrap();
            for candidate in pool {
                let id = candidate["id"].as_str().unwrap();
                let text = candidate["text"].as_str().unwrap();
                bank.commit(record(id, text)).await.unwrap();
                assert!(bank
                    .set_vector(id, MODEL, text, cache.exact(text).v.clone())
                    .await
                    .unwrap());
            }
            let (ranked, evidence) = bank
                .recall_evidence(query, Some(qv), MODEL, usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            for candidate in pool {
                let id = candidate["id"].as_str().unwrap();
                let e = evidence.get(id).expect("actual semantic candidate");
                assert!(e.vector && e.cosine.is_some_and(|c| c.is_finite()));
                if reasons.get(id).is_some_and(|r| r.supported()) != expected.contains(&id) {
                    failures.push(format!("{}/{index}/{id}: preselect", family["id"]));
                }
            }
            let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
            let response = serde_json::to_value(
                pre_generate(
                    &stack.deps,
                    input("neutral", query, "direct", "negative-proof"),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            rules(&response);
            assert_eq!(
                response["annotations"]["memory_retrieval"],
                "bm25-entity-semantic"
            );
            let preselected: Vec<_> = selected.iter().map(|(m, _)| m.id.as_str()).collect();
            if ids(&response) != json!(expected) || preselected != expected {
                failures.push(format!(
                    "{}/{index}: hook={}; select={preselected:?}",
                    family["id"],
                    ids(&response)
                ));
            }
            assert_eq!(stack.calls.lock().unwrap().as_slice(), [query]);
            clean(&stack);
            negatives += usize::from(expected.is_empty());
            executed += 1;
            println!(
                "{}",
                json!({"diagnostic":"minilm-neutral-proof","family":family["id"],"case":index,
                "candidate_vectors":pool.len(),"query_vector":true,"expected":expected,"selected":ids(&response),"lexical_fallback":false})
            );
        }
    }
    assert_eq!(executed, 20);
    assert_eq!(negatives, 16);
    println!(
        "{}",
        json!({"diagnostic":"minilm-neutral-total","executed":executed,"counterexamples":negatives,"new_inference":0})
    );
    assert!(failures.is_empty(), "neutral role controls: {failures:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn origin_privacy_rules_and_pins_remain_bound_with_exact_one_query() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::parse(MEMORIES, 0.0002);
    let query = cache.items["q-en-work-ledgerly-db"].text.clone();
    let stack = boundary(dir.path(), cache.clone()).await;
    let store = stack.deps.store().await;
    let (bank, _) = store.ensure_bank("negative-proof", None).await.unwrap();
    bank.set_rule("standing", RULE).unwrap();
    let cached = &cache.items["en-work-ledgerly-postgres"];
    let mut fact = record("en-work-ledgerly-postgres", &cached.text);
    fact.entities = vec!["Ledgerly".into()];
    fact.pinned = true;
    bank.commit(fact.clone()).await.unwrap();
    assert!(bank
        .set_vector(&fact.id, MODEL, &fact.text, cached.v.clone())
        .await
        .unwrap());
    let response = serde_json::to_value(
        pre_generate(
            &stack.deps,
            input("positive-control", &query, "direct", "negative-proof"),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(ids(&response), json!([fact.id]));
    assert_eq!(
        response["annotations"]["memory_retrieval"],
        "bm25-entity-semantic"
    );
    rules(&response);
    for kind in ["notification", "cron", "observer", "queued"] {
        let response = serde_json::to_value(
            pre_generate(&stack.deps, input(kind, &query, kind, "negative-proof"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ids(&response), json!([]));
        assert!(response["annotations"].get("memory_retrieval").is_none());
        no_facts_rendered(&response, std::slice::from_ref(&fact));
        rules(&response);
    }
    let mut later = input("positive-control", &query, "direct", "negative-proof");
    later.step = 1;
    let response = serde_json::to_value(pre_generate(&stack.deps, later).await.unwrap()).unwrap();
    assert_eq!(ids(&response), json!([]));
    let mut mismatched = input("correlation", &query, "direct", "negative-proof");
    mismatched.recall_context.as_mut().unwrap()["session_id"] = json!("another-session");
    let response =
        serde_json::to_value(pre_generate(&stack.deps, mismatched).await.unwrap()).unwrap();
    assert_eq!(ids(&response), json!([]));
    rules(&response);
    assert_eq!(
        stack.calls.lock().unwrap().as_slice(),
        [query.as_str()],
        "ineligible origins/later/mismatch never embed"
    );
    let (private, _) = store.ensure_bank("isolated-proof", None).await.unwrap();
    private.set_rule("standing", RULE).unwrap();
    let response = serde_json::to_value(
        pre_generate(
            &stack.deps,
            input("private-control", &query, "direct", "isolated-proof"),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ids(&response),
        json!([]),
        "named bank must not read other bank's pinned fact"
    );
    assert_eq!(
        response["annotations"]["memory_retrieval"],
        "bm25-entity-semantic"
    );
    rules(&response);
    no_facts_rendered(&response, std::slice::from_ref(&fact));
    assert_eq!(
        stack.calls.lock().unwrap().as_slice(),
        [query.as_str(), query.as_str()]
    );
    assert_eq!(bank.get(&fact.id).await.unwrap(), fact);
    clean(&stack);
    println!(
        "{}",
        json!({"diagnostic":"minilm-origin-privacy","eligible_queries":2,"suppressed_origin_cases":6,
        "private_zero_candidate_cases":1,"positive_vector_cases":1,"pin_unchanged":true,"provider_calls":0})
    );
}
