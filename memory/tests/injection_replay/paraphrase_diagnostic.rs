// Focused quality gate, not a new baseline. Original required facts and
// equivalent alternatives are checked in lexical and exact-cache lanes.
// A missing cached query measures lexical fallback, never embedder quality.
use super::*;

use super::evaluator::Labels;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relevance_r5_paraphrase_lexical_cached_comparison() {
    paraphrase_comparison(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slot_projection_cached_precision_first_policy_keeps_factual_loss_visible() {
    paraphrase_comparison(true).await;
}

async fn paraphrase_comparison(pilot_only: bool) {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let cases: [(&str, &[&[&str]]); 3] = [
        ("q-pt-life-par-mana", &[&["pt-life-julia-videochamada"]]),
        (
            "q-evt-next-week",
            &[
                &["evt-dentist-moved", "dup-dentist-moved"],
                &["evt-palestra-meetup"],
            ],
        ),
        (
            "q-en-work-release-days",
            &[
                &["en-work-deploy-window", "dup-deploy-window"],
                &["en-work-no-friday-deploys"],
            ],
        ),
    ];
    let fixture_now_ms = agenda::fixture_clock().now_ms();
    let known: HashSet<String> = corpus["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap().to_owned())
        .collect();
    let mut failures = Vec::new();
    let mut measured = 0;
    for (lane, raw) in [
        ("lexical", None),
        (
            "cached-a",
            Some(include_str!("../recall_quality/vectors.json")),
        ),
        (
            "cached-b",
            Some(include_str!("../recall_quality/vectors-minilm.json")),
        ),
    ] {
        let cache: Value = raw
            .map(|s| serde_json::from_str(s).unwrap())
            .unwrap_or_else(|| json!({"items":{},"model":""}));
        let dir = tempfile::tempdir().unwrap();
        let boundary = super::cached_boundary::boundary(dir.path(), cache.clone()).await;
        let deps = &boundary.deps;
        let mut cfg = (**deps.config.read().await).clone();
        cfg.embeddings_enabled = raw.is_some();
        *deps.config.write().await = Arc::new(cfg);
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("paraphrase-gate", None).await.unwrap();
        bank.set_rule("standing", "Preserve this diagnostic standing rule.")
            .unwrap();
        for fact in corpus["memories"].as_array().unwrap() {
            let id = fact["key"].as_str().unwrap();
            let text = fact["text"].as_str().unwrap();
            let at = fixture_now_ms - fact["age_days"].as_u64().unwrap() * 86_400_000;
            let m = Memory {
                id: id.into(),
                text: text.into(),
                entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
                tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
                pinned: fact["pinned"].as_bool().unwrap_or(false),
                confidence: crate::types::Confidence::Stated,
                corroboration: fact["corroboration"].as_u64().unwrap_or(0) as u32,
                source: None,
                created_at: at,
                updated_at: at,
                stated_at: Some(at),
                invalid_at: None,
                superseded_by: None,
                revision: 0,
            };
            bank.commit(m).await.unwrap();
            if raw.is_some() {
                let cached = &cache["items"][id];
                assert_eq!(cached["text"], text);
                let vector: Vec<f32> = serde_json::from_value(cached["v"].clone()).unwrap();
                assert_eq!(vector.len(), cache["dims"].as_u64().unwrap() as usize);
                assert!(vector.iter().all(|x| x.is_finite()));
                bank.set_vector(id, cache["model"].as_str().unwrap(), text, vector)
                    .await
                    .unwrap();
            }
        }
        for (query_id, groups) in cases {
            let pilot_pair = (lane == "cached-a" && query_id == "q-pt-life-par-mana")
                || (lane == "cached-b" && query_id == "q-en-work-release-days");
            if pilot_only && !pilot_pair {
                continue;
            }
            measured += 1;
            let text = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap()["text"]
                .as_str()
                .unwrap();
            let cached_query = cache["items"]
                .as_object()
                .unwrap()
                .values()
                .find(|v| v["text"] == text);
            let query_vector: Option<Vec<f32>> =
                cached_query.map(|v| serde_json::from_value(v["v"].clone()).unwrap());
            if let Some(v) = &query_vector {
                assert_eq!(v.len(), cache["dims"].as_u64().unwrap() as usize);
                assert!(v.iter().all(|n| n.is_finite()));
            }
            let exact = cached_query.is_some();
            if pilot_only {
                assert!(exact, "pilot precision needs an exact cached query");
            }
            // Original mandatory groups retained. No unrelated fact is made
            // admissible just because recall returned it.
            let original = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap();
            let labels = Labels {
                required: groups.iter().map(|g| g[0].to_owned()).collect(),
                forbidden: original["stale"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect(),
                equivalents: groups
                    .iter()
                    .filter(|g| g.len() > 1)
                    .map(|g| g.iter().map(|id| (*id).to_owned()).collect())
                    .collect(),
                ..Labels::default()
            };
            labels.validate(&known);
            let original_expected: HashSet<&str> = original["expect"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let grouped_expected: HashSet<&str> = groups
                .iter()
                .flat_map(|group| group.iter().copied())
                .collect();
            assert_eq!(
                grouped_expected, original_expected,
                "required alternatives must remain exactly those in the original retrieval fixture"
            );
            let session = format!("r5-{lane}-{query_id}");
            let input:PreGenerateInput=serde_json::from_value(json!({"session_id":session,"turn_id":format!("t-{session}"),"step":0,
                "metadata":{"memory_bank":"paraphrase-gate"},
                "recall_context":{"version":1,"session_id":session,"turn_id":format!("t-{session}"),"source":{"kind":"direct","entry_id":"current"},"query":{"entry_id":"current","text":text}},
                "generate":{"system_prompt":"offline quality comparison","messages":[{"role":"user","content":text}]}})).unwrap();
            let calls_before = boundary.calls.lock().unwrap().len();
            let response = pre_generate_at(deps, input, agenda::fixture_clock())
                .await
                .unwrap();
            let annotations = response.annotations.unwrap_or_default();
            let selected: Vec<String> = serde_json::from_value(
                annotations
                    .get("memory_ids")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            )
            .unwrap();
            assert_eq!(
                boundary.calls.lock().unwrap().len() - calls_before,
                usize::from(raw.is_some())
            );
            assert!(!boundary
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("UNEXPECTED:")));
            assert!(response
                .mutations
                .unwrap()
                .system_prompt
                .unwrap()
                .contains("Preserve this diagnostic standing rule."));
            let score = labels.score(&selected);
            let capped = crate::store::cap_query(text);
            let shared = bank.bm25_max(&capped, |_| true).await;
            let (ranked, evidence) = bank
                .recall_evidence_at(
                    &capped,
                    query_vector.as_deref(),
                    cache["model"].as_str().unwrap_or(""),
                    usize::MAX,
                    deps.config().await.decay_half_life_days,
                    false,
                    shared,
                    agenda::fixture_clock(),
                )
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            let supported: HashSet<String> = reasons
                .iter()
                .filter(|(_, reason)| reason.supported())
                .map(|(id, _)| id.clone())
                .collect();
            let policy = labels.injection_policy(&selected, &supported, exact);
            let admitted: Vec<&str> = ranked
                .iter()
                .filter(|(m, _)| {
                    evidence
                        .get(&m.id)
                        .is_some_and(|e| !e.matched.is_empty() || e.cosine.is_some())
                })
                .map(|(m, _)| m.id.as_str())
                .collect();
            let pool_cap = POOL_PER_SLOT.saturating_mul(deps.config().await.recall_limit);
            let mut examined: Vec<String> = groups
                .iter()
                .flat_map(|g| g.iter().map(|id| (*id).to_owned()))
                .collect();
            examined.extend(selected.iter().cloned());
            examined.extend(ranked.iter().take(8).map(|(m, _)| m.id.clone()));
            let mut once = HashSet::new();
            examined.retain(|id| once.insert(id.clone()));
            let candidate_evidence:Vec<Value>=examined.iter().map(|id| {
                let e=evidence.get(id);
                let pool_position=admitted.iter().position(|candidate|*candidate==id.as_str()).map(|n|n+1);
                let stage=if selected.contains(id){"selected_by_real_hook"}
                    else if e.is_none(){"not_a_recall_candidate"}
                    else if e.is_some_and(|e|e.matched.is_empty() && e.cosine.is_none()){"no_admission_evidence"}
                    else if pool_position.is_some_and(|n|n>pool_cap){"outside_bounded_evidence_pool"}
                    else if e.is_some_and(|e|e.subject_conflict){"subject_conflict_observed"}
                    else if e.is_some_and(|e|e.attribute_conflict){"attribute_conflict_observed"}
                    else if e.is_some_and(|e|e.predicate_conflict){"predicate_conflict_observed"}
                    else {"present_not_selected_reason_not_inferred"};
                json!({"id":id,"required_alternative":groups.iter().any(|g|g.contains(&id.as_str())),"selected":selected.contains(id),
                    "rank":ranked.iter().position(|(m,_)|m.id==*id).map(|n|n+1),"pool_position":pool_position,"pool_cap":pool_cap,"diagnostic_stage":stage,
                    "matched":e.map(|e|&e.matched),"terms":e.map(|e|e.terms),
                    "relation_matched":e.map(|e|&e.relation_matched),"relation_terms":e.map(|e|e.relation_terms),
                    "admission_reason":reasons.get(id).copied().unwrap_or(AdmissionReason::NoIndependentRelation),
                    "target_scope":e.map(|e|json!({"required":e.target_scope_required,"matched":e.target_scope_matched,
                        "kind":e.target_scope_kind,"span":e.target_scope_span,"terms":e.target_scope_terms,
                        "predicate":e.target_scope_predicate,"subject":e.target_scope_subject,"object":e.target_scope_object,"relative":e.target_scope_relative})),
                    "requested_slot_matched":e.map(|e|&e.requested_slot_matched),
                    "requested_slot_spans":e.and_then(|e|e.requested_slot_spans.as_ref().map(|spans|spans.iter().map(|s|[s.start,s.end]).collect::<Vec<_>>())),
                    "projection_unknown":e.map(|e|e.requested_slot_spans.is_none()),"lexical":e.map(|e|e.lexical),"relevance":e.map(|e|e.relevance),"semantic_contribution":e.map(|e|e.relevance-e.lexical),
                    "cosine":e.and_then(|e|e.cosine),"z":e.and_then(|e|e.z),"vector":e.map(|e|e.vector),
                    "detail_match":e.map(|e|e.detail_match),"relation_detail_match":e.map(|e|e.relation_detail_match),
                    "attribute":e.map(|e|e.attribute_match),"attribute_conflict":e.map(|e|e.attribute_conflict),"personal":e.map(|e|e.personal_match),
                    "personal_scope_spans":e.map(|e|&e.personal_scope_spans),"personal_scope_terms":e.map(|e|&e.personal_scope_terms),
                    "personal_scope_predicates":e.map(|e|&e.personal_scope_predicates),"personal_scope_owners":e.map(|e|&e.personal_scope_owners),
                    "personal_scope_matched":e.map(|e|e.personal_scope_matched),"personal_scope_required":e.map(|e|e.personal_scope_required),
                    "named_subject":e.map(|e|e.named_subject),"subject_conflict":e.map(|e|e.subject_conflict),"predicate_conflict":e.map(|e|e.predicate_conflict)})
            }).collect();
            println!(
                "{}",
                json!({"diagnostic":"paraphrase-lexical-cached-comparison","query_id":query_id,"lane":lane,"text":text,"required_groups":groups,
                "selected":selected,"evaluation":score,"injection_policy":policy,
                "status":if policy.integral_ok{"policy_pass"}else{"quality_red"},"candidate_evidence":candidate_evidence,
                "stage_scope":"observed evidence and real-hook selection; score-free support audited before selection",
                "policy_decision":"similarity discovers/ranks; independent relation authorizes; factual recall unchanged",
                "exact_query_cache_present":exact,"embedding_quality_measured":raw.is_some() && exact,"fallback_measured":raw.is_none() || !exact,
                "provider_calls":0,"generation_calls":0,"monetary_cost":null,"coverage_complete":false,
                "fixture_now_ms":fixture_now_ms,"calendar_validity_measured":true,
                "calendar_scope":if query_id=="q-evt-next-week"{"synthetic 2026-10-12 UTC: unchanged October events are next week; fixed 2026-11-16 Berlin trip is outside [2026-10-19,2026-10-26)"}else{"calendar reference not evaluated"}})
            );
            if pilot_only {
                // The precision-first policy may abstain on this historically
                // semantic-only pair. Never report factual recall as recovered.
                assert!(policy.integral_ok, "{lane}/{query_id}: {policy:?}");
                assert!(
                    selected.iter().all(|id| supported.contains(id)),
                    "{selected:?}"
                );
                assert!(score.duplicate_equivalents.is_empty(), "{score:?}");
            }
            if !policy.integral_ok {
                failures.push(format!("{lane}/{query_id}"));
            }
        }
    }
    assert_eq!(measured, if pilot_only { 2 } else { 9 });
    assert!(
        failures.is_empty(),
        "unresolved independent support policy (supported miss, noise or duplicates); factual labels remain unchanged: {failures:?}"
    );
}
