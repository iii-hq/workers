// Post-contract replay through the real pre_generate hook and inert SDK
// transport. Exact cached queries use stored vectors; misses use lexical
// fallback. Historical raw-query baseline receipts remain separate evidence.
// No live embedder, generation, extraction or billing is exercised.
use super::*;
use std::collections::BTreeMap;
use std::time::Instant;

use super::evaluator::Labels;

#[derive(Deserialize)]
struct Corpus {
    memories: Vec<Fact>,
    queries: Vec<Query>,
    negatives: Vec<Query>,
}

#[derive(Deserialize)]
struct Fact {
    key: String,
    text: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    corroboration: u32,
    age_days: u64,
}

#[derive(Deserialize)]
struct Query {
    id: String,
    text: String,
    #[serde(default)]
    expect: Vec<String>,
    #[serde(default)]
    stale: Vec<String>,
}

#[derive(Deserialize)]
struct Cache {
    model: String,
    dims: usize,
    items: BTreeMap<String, Cached>,
}

#[derive(Deserialize)]
struct Cached {
    text: String,
    v: Vec<f32>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    split: String,
    lang: String,
    kind: String,
    #[serde(default)]
    text: String,
    query_id: Option<String>,
    prior: Option<String>,
    #[serde(default)]
    seen: Vec<String>,
    #[serde(default)]
    layers: Vec<String>,
    budget: Option<u64>,
    #[serde(default)]
    later_step: bool,
    #[serde(flatten)]
    labels: Labels,
    #[serde(default)]
    assert_pinned: Vec<String>,
}

fn fact(id: &str, text: &str, entities: &[&str], at: u64) -> Memory {
    Memory {
        id: id.into(),
        text: text.into(),
        entities: entities.iter().map(|s| s.to_string()).collect(),
        tags: Vec::new(),
        confidence: crate::types::Confidence::Stated,
        corroboration: 0,
        pinned: false,
        source: None,
        created_at: at,
        updated_at: at,
        stated_at: Some(at),
        invalid_at: None,
        superseded_by: None,
        revision: 0,
    }
}

fn deps_at(dir: &std::path::Path, cfg: WorkerConfig) -> Deps {
    // Deliberately never connect this inert client. Embeddings are disabled.
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
    let store = crate::store::Store::open(dir.to_path_buf()).unwrap();
    Deps {
        emitter: Arc::new(crate::events::Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(store))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
    }
}

// The evaluator reads only production hook ids, holders and rendered bytes.
// Consumer fixtures do not prove producer authority; that is tested separately.
#[allow(clippy::too_many_arguments)]
async fn hook_replay(
    deps: &Arc<Deps>,
    session: &str,
    c: &Case,
    text: &str,
    window: &Value,
    facts: &HashMap<String, Memory>,
    root: &std::path::Path,
) -> Injection {
    let mut context = json!({"version":1,"session_id":session,"turn_id":format!("t-{session}"),
        "source":{"kind":"direct","entry_id":"current"},"query":{"entry_id":"current","text":text}});
    if let Some(prior) = &c.prior {
        context["continuation"] = json!({"entry_id":"prior","text":prior});
    }
    let mut input: PreGenerateInput = serde_json::from_value(
        json!({"session_id":session,"turn_id":format!("t-{session}"),"step":u64::from(c.later_step),
        "metadata":{"memory_bank":"replay-mixed"},"recall_context":context,
        "generate":{"system_prompt":"offline replay","messages":window}}),
    )
    .unwrap();
    if !c.layers.is_empty() {
        input.metadata = Some(json!({"fs_scope":{"root":root}}));
    }
    let response = pre_generate(deps, input).await.unwrap();
    let annotations = response.annotations.unwrap_or_default();
    let ids: Vec<String> = serde_json::from_value(
        annotations
            .get("memory_ids")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .unwrap();
    let banks: Vec<String> = serde_json::from_value(
        annotations
            .get("memory_banks")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .unwrap();
    assert_eq!(ids.len(), banks.len());
    let mutations = response.mutations.unwrap_or_default();
    let message = mutations
        .append_messages
        .iter()
        .map(|m| crate::extract::content_text(m.get("content")))
        .collect::<Vec<_>>()
        .join("\n");
    Injection {
        section: mutations.system_prompt.unwrap_or_default(),
        rules: annotations
            .get("memory_rules")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        rules_truncated: false,
        memories: ids
            .into_iter()
            .zip(banks)
            .map(|(id, b)| (facts[&id].clone(), 0.0, b))
            .collect(),
        message: (!message.is_empty()).then_some(message),
        retrieval: if annotations.contains_key("memory_retrieval") {
            "bm25-entity-semantic"
        } else {
            "bm25-entity"
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synthetic_injection_replay_dev_holdout() {
    let cases_raw = include_str!("cases.json");
    let golden_raw = include_str!("../recall_quality/golden.json");
    let corpus: Corpus = serde_json::from_str(golden_raw).unwrap();
    let cases: Vec<Case> = serde_json::from_str(cases_raw).unwrap();
    let mut ids = HashSet::new();
    for c in &cases {
        assert!(ids.insert(&c.id), "duplicate scenario {}", c.id);
        assert!(["dev", "holdout"].contains(&c.split.as_str()));
        assert!(["pt", "en"].contains(&c.lang.as_str()));
        assert!(
            c.query_id.is_some() ^ !c.text.is_empty(),
            "one query source required"
        );
        assert!(c.labels.required.iter().all(|k| !k.is_empty()));
    }
    let at = now_ms();
    let mut failures = Vec::new();
    let mut factual_failures = Vec::new();
    let mut relevance_failures = Vec::new();
    let mut temporal_dependencies = Vec::new();
    let mut measured = 0;
    let mut skipped = 0;
    // The cache model is provenance only. This does not choose an active route.
    for (lane, raw) in [
        ("lexical-production", None),
        (
            "hybrid-cached-a",
            Some(include_str!("../recall_quality/vectors.json")),
        ),
        (
            "hybrid-cached-b",
            Some(include_str!("../recall_quality/vectors-minilm.json")),
        ),
    ] {
        let cache: Option<Cache> = raw.map(|s| serde_json::from_str(s).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let base_cfg = WorkerConfig {
            default_bank: "replay-global".into(),
            embeddings_enabled: false,
            extraction_enabled: false,
            recall_limit: 6,
            recall_budget_tokens: 1200,
            ..WorkerConfig::default()
        };
        let boundary = super::cached_boundary::boundary(
            dir.path(),
            raw.map(|s| serde_json::from_str(s).unwrap())
                .unwrap_or_else(|| json!({"items":{},"model":""})),
        )
        .await;
        let deps = boundary.deps.clone();
        *deps.config.write().await = Arc::new(base_cfg.clone());
        let store = deps.store().await;
        let bank_name = crate::scope::root_bank(dir.path()).unwrap();
        let (bank, _) = store.ensure_bank("replay-mixed", None).await.unwrap();
        let (layered_bank, _) = store.ensure_bank(&bank_name, None).await.unwrap();
        layered_bank
            .set_rule(
                "standing",
                "Responda com clareza; preserve regras permanentes.",
            )
            .unwrap();
        bank.set_rule(
            "standing",
            "Responda com clareza; preserve regras permanentes.",
        )
        .unwrap();
        let mut facts = HashMap::new();
        for f in &corpus.memories {
            let m = Memory {
                entities: f.entities.clone(),
                tags: f.tags.clone(),
                pinned: f.pinned,
                corroboration: f.corroboration,
                ..fact(&f.key, &f.text, &[], at - f.age_days * 86_400_000)
            };
            assert!(facts.insert(f.key.clone(), m.clone()).is_none());
            bank.commit(m.clone()).await.unwrap();
            layered_bank.commit(m.clone()).await.unwrap();
            if let Some(cache) = &cache {
                let v = cache
                    .items
                    .get(&f.key)
                    .expect("memory cache missing; never re-embed");
                assert_eq!(v.text, f.text, "cache text mismatch");
                assert_eq!(v.v.len(), cache.dims);
                assert!(v.v.iter().all(|x| x.is_finite()));
                bank.set_vector(&m.id, &cache.model, &m.text, v.v.clone())
                    .await
                    .unwrap();
                layered_bank
                    .set_vector(&m.id, &cache.model, &m.text, v.v.clone())
                    .await
                    .unwrap();
            }
        }
        for (name, id, text, entities) in [
            (
                "replay-global",
                "layer-user",
                "O usuário prefere formato marzipan em tópicos.",
                vec!["user", "marzipan"],
            ),
            (
                "replay-global",
                "layer-hidden",
                "The legacy service uses marzipan format.",
                vec!["legacy", "marzipan"],
            ),
            (
                "agent-replay",
                "layer-agent",
                "My agent uses marzipan format with numbered steps.",
                vec!["marzipan"],
            ),
            (
                "replay-foreign",
                "foreign-only",
                "The foreign-only kumquat access code is synthetic-123.",
                vec!["kumquat"],
            ),
        ] {
            let name = if name == "agent-replay" {
                crate::scope::agent_bank("replay")
            } else {
                name.to_string()
            };
            let (layer, _) = store.ensure_bank(&name, None).await.unwrap();
            let m = fact(id, text, &entities, at);
            facts.insert(id.into(), m.clone());
            layer.commit(m).await.unwrap();
            if name == "replay-global" {
                layer
                    .set_rule("standing", "Não omita as regras do layer.")
                    .unwrap();
            }
        }
        for c in &cases {
            c.labels.validate(&facts.keys().cloned().collect());
            for key in c.seen.iter().chain(&c.assert_pinned) {
                assert!(facts.contains_key(key), "unknown fixture key {key}");
            }
            let text = c
                .query_id
                .as_ref()
                .map(|id| {
                    corpus
                        .queries
                        .iter()
                        .chain(&corpus.negatives)
                        .find(|q| &q.id == id)
                        .expect("unknown cached query")
                        .text
                        .as_str()
                })
                .unwrap_or(&c.text);
            let effective = c.prior.as_deref().unwrap_or(text);
            let qv = cache
                .as_ref()
                .and_then(|v| v.items.values().find(|q| q.text == effective));
            if let Some(v) = qv {
                assert_eq!(v.text, effective);
                assert_eq!(v.v.len(), cache.as_ref().unwrap().dims);
            } else if cache.is_some() {
                skipped += 1;
            }
            let mut messages = Vec::new();
            if let Some(prior) = &c.prior {
                messages.push(json!({"role":"user","content":prior}));
                messages.push(json!({"role":"assistant","content":"I can continue that topic."}));
            }
            if !c.seen.is_empty() {
                let shown: Vec<Memory> = c.seen.iter().map(|k| facts[k].clone()).collect();
                let body = memories_message("replay-mixed", &shown, 1200, at)
                    .unwrap()
                    .0;
                messages.push(json!({"role":"user","content":body}));
            }
            messages.push(json!({"role":"user","content":text}));
            let window = Value::Array(messages);
            let query = last_user_text(Some(&window));
            assert_eq!(query, text);
            let seen = injected_in_window(Some(&window));
            let cfg = WorkerConfig {
                default_bank: if c.layers.len() == 1 && c.layers[0] == "agent-replay" {
                    "replay-empty-global".into()
                } else {
                    base_cfg.default_bank.clone()
                },
                recall_budget_tokens: c.budget.unwrap_or(1200),
                ..base_cfg.clone()
            };
            let session = format!("{lane}-{}", c.id);
            let calls_before = boundary.calls.lock().unwrap().len();
            if !c.layers.is_empty() {
                boundary.sessions.lock().unwrap().insert(
                    session.clone(),
                    if c.layers.contains(&"agent-replay".to_string()) {
                        json!({"agent_profile":{"id":"replay"}})
                    } else {
                        json!({})
                    },
                );
            }
            *deps.config.write().await = Arc::new(WorkerConfig {
                embeddings_enabled: cache.is_some(),
                ..cfg.clone()
            });
            let mut times = Vec::new();
            let mut baseline: Option<Vec<String>> = None;
            let mut final_out = Injection::none();
            // One warmup, five measured sequential calls on identical data.
            for repetition in 0..6 {
                let start = Instant::now();
                let out = hook_replay(&deps, &session, c, text, &window, &facts, dir.path()).await;
                let us = start.elapsed().as_micros();
                let selected: Vec<String> =
                    out.memories.iter().map(|(m, _, _)| m.id.clone()).collect();
                if let Some(first) = &baseline {
                    assert_eq!(&selected, first, "unstable replay {}", c.id);
                } else {
                    baseline = Some(selected);
                }
                if repetition > 0 {
                    times.push(us);
                }
                final_out = out;
            }
            times.sort_unstable();
            measured += 1;
            let embedding_calls = boundary.calls.lock().unwrap().len() - calls_before;
            assert_eq!(
                embedding_calls,
                // Independent fixture intent, not receipt_only() or another
                // production helper: these two literal scenarios request only
                // receipt, while all other opening-step scenarios are eligible.
                if cache.is_some()
                    && !c.later_step
                    && !matches!(
                        c.id.as_str(),
                        "dev-technical-event-pt" | "holdout-technical-event-en"
                    )
                {
                    6
                } else {
                    0
                },
                "unexpected embedding boundary calls for {}",
                c.id
            );
            assert!(!boundary
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.starts_with("UNEXPECTED:")));
            let selected = baseline.unwrap();
            let score = c.labels.score(&selected);
            let mut audit_hits = Vec::new();
            let mut audit_evidence = HashMap::new();
            let sources = if c.layers.is_empty() {
                vec![("replay-mixed".to_owned(), false)]
            } else {
                std::iter::once((bank_name.clone(), false))
                    .chain(c.layers.iter().map(|name| {
                        (
                            if name == "agent-replay" {
                                crate::scope::agent_bank("replay")
                            } else {
                                name.clone()
                            },
                            true,
                        )
                    }))
                    .collect()
            };
            for (holder_name, layered) in sources {
                let (holder, _) = store.ensure_bank(&holder_name, None).await.unwrap();
                let (found, evidence) = holder
                    .recall_evidence(
                        effective,
                        qv.map(|q| q.v.as_slice()),
                        cache.as_ref().map_or("", |q| q.model.as_str()),
                        usize::MAX,
                        cfg.decay_half_life_days,
                        false,
                        0.0,
                    )
                    .await;
                for (m, score) in found {
                    if !layered || crate::scope::layer_keeps(&holder_name, &cfg.default_bank, &m) {
                        audit_evidence.insert(m.id.clone(), evidence[&m.id].clone());
                        audit_hits.push((m, score));
                    }
                }
            }
            let audit_hits = merge_ranked(audit_hits, usize::MAX);
            let reasons = admission_reasons(&audit_hits, &audit_evidence);
            let supported = reasons
                .iter()
                .filter(|(_, reason)| reason.supported())
                .map(|(id, _)| id.clone())
                .collect();
            let policy = c
                .labels
                .injection_policy(&selected, &supported, qv.is_some());
            let mut update_proofs = Vec::new();
            for q in &corpus.queries {
                for old in &q.stale {
                    for current in &q.expect {
                        let (Some(old_fact), Some(new_fact)) = (facts.get(old), facts.get(current))
                        else {
                            continue;
                        };
                        let source = format!("original golden query {} expected/stale + shared entity/tag + statement order", q.id);
                        if let Some(proof) = super::evaluator::UpdateProof::from_original(
                            old_fact, new_fact, &q.expect, &q.stale, &source,
                        ) {
                            update_proofs.push(proof);
                        }
                    }
                }
            }
            let responsibility = c.labels.responsibility(&selected, &update_proofs);
            let temporal_only = responsibility.temporal_only;
            for key in &c.assert_pinned {
                assert!(facts[key].pinned, "pinned fixture lost its flag");
                assert!(
                    bank.get(key).await.unwrap().pinned,
                    "store changed pinned flag"
                );
            }
            let line_bytes: usize = final_out
                .memories
                .iter()
                .map(|(m, _, _)| {
                    format!(
                        "- {} ({})",
                        memory_line_text(m),
                        local_date(m.last_stated(), "%Y-%m-%d")
                    )
                    .len()
                })
                .sum();
            let message_bytes = final_out.message.as_ref().map_or(0, String::len);
            let ok = policy.integral_ok;
            let case_key = format!("{lane}/{}", c.id);
            if !score.ok {
                factual_failures.push(case_key.clone());
            }
            if !ok {
                if temporal_only {
                    temporal_dependencies.push(case_key.clone());
                } else {
                    relevance_failures.push(case_key);
                }
            }
            // Structural contracts stay strict even when a quality case is red.
            assert!(line_bytes <= cfg.recall_budget_tokens as usize * 4);
            assert!(final_out.section.contains("Responda com clareza"));
            assert!(final_out.memories.iter().all(|(m, _, holder)| {
                !seen.contains(&memory_line_text(m))
                    && (holder == "replay-mixed"
                        || holder == &bank_name
                        || holder == "replay-global"
                        || holder == &crate::scope::agent_bank("replay"))
            }));
            if !c.layers.is_empty() && c.layers.contains(&"replay-global".to_string()) {
                assert!(final_out.section.contains("Não omita as regras do layer"));
            }
            println!(
                "{}",
                json!({
                    "case":c.id,"split":c.split,"lang":c.lang,"kind":c.kind,"lane":lane,
                    "embedding_quality_measured":cache.is_some() && qv.is_some() && !c.later_step,
                    "embedding_cache_miss":cache.is_some() && qv.is_none() && !c.later_step,
                    "status":if ok {"pass"} else {"quality_red"},"selected":selected,
                    "holders":final_out.memories.iter().map(|(_,_,b)|b).collect::<Vec<_>>(),
                    "evaluation":score,"injection_policy":policy,"responsibility_proof":responsibility,
                    "admission_reasons":reasons,
                    "responsibility":if temporal_only {"consolidation_temporal_dependency_not_global_pass"} else if !ok {"relevance_or_precision"} else {"satisfied"},
                    "labels":{"required":c.labels.required,"admissible":c.labels.admissible,"equivalents":c.labels.equivalents},
                    "abstained":selected.is_empty(),
                    "query_origin":if c.query_id.as_ref().is_some_and(|id| id.starts_with("neg-")) {"existing-negative-text; context-specific labels"} else if c.query_id.is_some() {"existing-golden-query"} else {"novel-scenario-text"},
                    "trusted_technical_origin_covered":false,
                    "orchestration":"real pre_generate + inert SDK boundary; producer authority separately tested",
                    "effective_query":effective,"comparison":"post-contract; historical raw-query lanes not overwritten",
                    "memory_line_bytes":line_bytes,"appended_message_bytes":message_bytes,
                    "section_bytes":final_out.section.len(),"budget_estimated_tokens":cfg.recall_budget_tokens,
                    "message_over_line_budget":message_bytes > cfg.recall_budget_tokens as usize * 4,
                    "latency_us_samples":times,"latency_us_p50":times[2],"latency_us_p95_nearest_rank":times[4],
                    "embedding_calls":embedding_calls,"embedding_provider_calls":0,"generation_calls":0,"monetary_cost":null,
                    "query_vector_present":qv.is_some(),"retrieval":final_out.retrieval,
                    "cache_model":cache.as_ref().map(|v| v.model.as_str()),
                    "cache_dims":cache.as_ref().map(|v| v.dims),
                    "cache_fingerprint":raw.map(crate::types::fingerprint)
                })
            );
            if !ok {
                let (ranked, ev) = bank
                    .recall_evidence(
                        effective,
                        qv.map(|q| q.v.as_slice()),
                        cache.as_ref().map_or("", |q| q.model.as_str()),
                        8,
                        cfg.decay_half_life_days,
                        false,
                        0.0,
                    )
                    .await;
                println!(
                    "{}",
                    json!({"diagnostic":"quality-residue","lane":lane,"case":c.id,
                    "candidates":ranked.iter().map(|(m,_)| {let e=&ev[&m.id];json!({"id":m.id,"matched":e.matched,"terms":e.terms,"named":e.named_subject,"detail":e.detail_match,"specific":e.specific_recall,"personal":e.personal_match,"subject_question":e.subject_question,"attribute":e.attribute_match,"attribute_conflict":e.attribute_conflict,"predicate_conflict":e.predicate_conflict,"subject_conflict":e.subject_conflict,"lexical":e.lexical,"relevance":e.relevance,"cosine":e.cosine,"z":e.z})}).collect::<Vec<_>>() })
                );
                failures.push(format!("{lane}/{}", c.id));
            }
        }
    }
    println!(
        "{}",
        json!({"corpus_fingerprint":crate::types::fingerprint(cases_raw),
        "bank_fingerprint":crate::types::fingerprint(golden_raw),"fixture_now_ms":at,
        "replay_source_fingerprint":crate::types::fingerprint(include_str!("replay.rs")),
        "scenario_count":cases.len(),"measured_hook_lane_cases":measured,"embedding_quality_not_measured_lane_cases":skipped,
        "quality_failures":failures,"factual_quality_failures":factual_failures,
        "relevance_failures":relevance_failures,"temporal_dependencies":temporal_dependencies,
        "coverage_complete":false,
        "scope_classification":"relevance/abstention misses remain blocking; forbidden stale facts and equivalent duplicates only are temporal/consolidation dependencies, never counted as pass",
        "coverage_gaps":["producer authority tested separately", "effective continuation/novel query without exact cache uses lexical fallback, not measured embedding quality"],
        "evaluator_source_fingerprint":crate::types::fingerprint(include_str!("evaluator.rs")),
        "clock":"real wall clock for decay/date; Instant for local latency",
        "holdout_scope":"scenario labels only; shared synthetic bank and pre-existing query cache, not unseen data",
        "latency_scope":"warm hook replay including in-process SDK/cache transport; excludes seeding and live embedder/model generation"})
    );
    assert!(
        failures.is_empty(),
        "quality red (do not rebaseline or weaken labels): {failures:?}"
    );
}

#[tokio::test]
async fn legacy_origin_claims_are_rules_only() {
    // Actual producer: harness/src/hooks/runner.rs envelope + generate.
    // Metadata and message values are opaque, not authenticated provenance.
    // Legacy arbitrary origin claims cannot opt into the controlled contract.
    let schema = serde_json::to_value(schemars::schema_for!(PreGenerateInput)).unwrap();
    let properties = schema["properties"].as_object().unwrap();
    assert!(!properties.contains_key("origin"));
    assert!(!properties.contains_key("source"));
    assert!(properties.contains_key("recall_context"));
    let dir = tempfile::tempdir().unwrap();
    let cfg = WorkerConfig {
        embeddings_enabled: false,
        extraction_enabled: false,
        recall_limit: 6,
        ..WorkerConfig::default()
    };
    let deps = Arc::new(deps_at(dir.path(), cfg));
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("origin-proof", None).await.unwrap();
    let mut pinned = fact(
        "origin-pin",
        "Marzipan recall has a permanent pinned fact.",
        &["marzipan"],
        now_ms(),
    );
    pinned.pinned = true;
    bank.commit(pinned).await.unwrap();
    bank.set_rule("standing", "Preserve this permanent rule.")
        .unwrap();
    let mut responses = Vec::new();
    for (n, claimed) in ["human", "technical-notification"].iter().enumerate() {
        let raw = json!({
            "point":"pre_generate", "session_id":format!("replay-origin-{n}"), "step":0,
            "origin":{"kind":claimed},
            "metadata":{"memory_bank":"origin-proof", "origin":{"kind":claimed}},
            "generate":{"system_prompt":"Offline test", "messages":[
                {"role":"user", "content":"Marzipan recall", "origin":{"kind":claimed}}
            ]}
        });
        let input: PreGenerateInput = serde_json::from_value(raw).unwrap();
        // No text or regex is used to choose a trusted event class.
        responses.push(serde_json::to_value(pre_generate(&deps, input).await.unwrap()).unwrap());
    }
    assert_eq!(responses[0]["annotations"]["memory_ids"], Value::Null);
    assert_eq!(
        responses[0]["annotations"]["memory_ids"],
        responses[1]["annotations"]["memory_ids"]
    );
    // Rules-only has no appended recall; no volatile timestamp normalization
    // is needed now. Compare the full responses so rules/annotations cannot drift.
    assert_eq!(responses[0], responses[1]);
    assert!(responses
        .iter()
        .all(|r| r["mutations"].get("append_messages").is_none()));
    assert!(responses[0]["mutations"]["system_prompt"]
        .as_str()
        .unwrap()
        .contains("Preserve this permanent rule"));
    assert!(bank.get("origin-pin").await.unwrap().pinned);
    println!(
        "{}",
        json!({"check":"legacy-origin-claims", "status":"rules-only",
        "producer_authority_proven_by_this_test":false,
        "observed":"legacy top-level/metadata/message origin claims neither authenticate nor enable recall"})
    );
}
