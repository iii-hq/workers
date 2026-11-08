use super::*;

#[path = "technical_relations.rs"]
mod technical_relations;

// These assertions label factual requirements independently of admission reasons.
// They exercise the actual Store and pre-generate consumer, never an evaluator gate.

#[tokio::test]
async fn temporal_eligibility_binds_owner_action_and_complete_calendar() {
    let first =
        "User ships to production only on Mondays and Wednesdays, between 08:15 and 12:45 UTC.";
    let twin = "User deploys to production only on Monday and Wednesday, from 08:15 to 12:45 UTC.";
    let exclusion = "User never releases on Saturdays or on the day before a public holiday, to avoid unattended incidents.";
    assert!(crate::extract::same_fact(first, twin));
    assert!(crate::extract::same_fact(twin, first));
    for other in [
        "User deploys to production only on Monday and Wednesday, from 08:15 to 12:46 UTC.",
        "User deploys to production only on Monday and Wednesday, from 08:15 to 12:45 local time.",
        "User deploys to production only on Monday and Friday, from 08:15 to 12:45 UTC.",
        "User deploys to production only on Monday and Wednesday, from 08:15 to 12:45 UTC, with a reviewer.",
        "The visitor deploys to production only on Monday and Wednesday, from 08:15 to 12:45 UTC.",
        "User's colleague deploys to production only on Monday and Wednesday, from 08:15 to 12:45 UTC.",
        "User does not deploy to production only on Monday and Wednesday, from 08:15 to 12:45 UTC.",
    ] {
        assert!(!crate::extract::same_fact(first, other), "{other}");
        assert!(!crate::extract::same_fact(other, first), "{other}");
    }
    let negatives = [
        "The visitor never releases on Saturdays or on the day before a public holiday.",
        "User's colleague never releases on Saturdays or on the day before a public holiday.",
        "User reports that the visitor never releases on Saturdays.",
        "If user never releases on Saturdays, ask the visitor.",
        "User quotes: user never releases on Saturdays.",
        "Saturdays release user to production only on Monday and Wednesday.",
        "User writes release files on Mondays and Wednesdays, between 08:15 and 12:45 UTC.",
    ];
    for query in [
        "Which weekdays am I allowed to deploy to production, and during what hours?",
        "Which days am I allowed to ship to production?",
        "Thinking of putting a change out at 5pm on the last workday of the week, any reason I shouldn't?",
        "I am deploying to production at 08:15 on Monday, am I allowed to do so?",
        "Shipping to production at 11:42am on the first workday of the week, any reason I should not?",
        "Considering putting a change out at 23:59 on Sunday, am I allowed to do that?",
    ] {
        for (index, text) in [first, twin, exclusion]
            .into_iter()
            .chain(negatives)
            .enumerate()
        {
            let expected = index < 3;
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = record("calendar", text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            assert_eq!(
                evidence
                    .get("calendar")
                    .and_then(|e| e.temporal_constraint.as_ref())
                    .is_some(),
                expected,
                "{query} / {text}"
            );
            assert_eq!(
                select(ranked, &evidence, &HashSet::new(), 6, query).len(),
                usize::from(expected),
                "{query} / {text}"
            );
            let response = hook(
                &deps,
                "calendar",
                json!([{"role":"user","content":query}]),
                context("calendar", "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["calendar"])
                } else {
                    json!([])
                },
                "{query} / {text}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("calendar").await.unwrap(), original);
        }
    }
    // All roles use the same words, but a foreign possessor or an unrelated
    // document operation cannot request the user's production calendar.
    for query in [
        "Which weekdays is the visitor allowed to deploy to production, and during what hours?",
        "Which weekdays am I allowed to release a document, and during what hours?",
        "Which weekdays am I allowed to deploy to my colleague's production, and during what hours?",
        "The visitor is deploying to production at 5pm on Friday, any reason I shouldn't?",
        "Thinking of putting a document out at 5pm on the last workday of the week, any reason I shouldn't?",
        "Thinking of not putting a change out at 5pm on the last workday of the week, any reason I shouldn't?",
        "If I am deploying to production at 5pm on Friday, am I allowed to do so?",
        "User quotes: deploying to production at 5pm on Friday, am I allowed to do so?",
        "Deploying to production at 25:00 on Monday, am I allowed to do so?",
        "Deploying to production at 00pm on Friday, am I allowed to do so?",
    ] {
        assert!(crate::store::query_projection::temporal_constraint(query, first).is_none());
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(record("calendar", first)).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.").unwrap();
        let (ranked, evidence) = bank.recall_evidence(query, None, "", usize::MAX, 30, false, 0.0).await;
        assert!(evidence.values().all(|e| e.temporal_constraint.is_none()));
        let response = hook(&deps, "foreign-calendar", json!([{"role":"user","content":query}]), context("foreign-calendar", "direct", query)).await;
        // A rejected temporal frame must not borrow another owner through
        // literal overlap in either Store selection or product injection.
        assert!(select(ranked, &evidence, &HashSet::new(), 6, query).is_empty(), "{query}");
        assert_eq!(ids(&response), json!([]), "{query}");
        rules_preserved(&response);
    }
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    for (id, text) in [
        ("window", first),
        ("paraphrase", twin),
        ("exclusion", exclusion),
    ] {
        bank.commit(record(id, text)).await.unwrap();
    }
    let query = "Which weekdays am I allowed to deploy to production, and during what hours?";
    let response = hook(
        &deps,
        "calendar-pair",
        json!([{"role":"user","content":query}]),
        context("calendar-pair", "direct", query),
    )
    .await;
    let selected = ids(&response);
    let selected = selected.as_array().unwrap();
    assert_eq!(selected.len(), 2);
    assert!(selected.contains(&json!("exclusion")));
    assert_eq!(
        selected
            .iter()
            .filter(|id| **id == json!("window") || **id == json!("paraphrase"))
            .count(),
        1
    );
    rules_preserved(&response);
    for id in ["window", "paraphrase", "exclusion"] {
        assert!(bank.get(id).await.unwrap().is_live());
    }
}

#[tokio::test]
async fn integrated_constraints_keep_original_factual_requirements_without_policy_certification() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    for fact in corpus["memories"].as_array().unwrap() {
        let mut m = record(
            fact["key"].as_str().unwrap(),
            fact["text"].as_str().unwrap(),
        );
        let at = now_ms().saturating_sub(fact["age_days"].as_u64().unwrap() * 86_400_000);
        m.created_at = at;
        m.updated_at = at;
        m.stated_at = Some(at);
        m.entities = serde_json::from_value(fact["entities"].clone()).unwrap();
        m.tags = serde_json::from_value(fact["tags"].clone()).unwrap();
        m.pinned = fact["pinned"].as_bool().unwrap_or(false);
        m.corroboration = fact["corroboration"].as_u64().unwrap_or(0) as u32;
        bank.commit(m).await.unwrap();
    }
    let before = std::fs::read(dir.path().join("hook-relevance/memories.jsonl")).unwrap();
    for (query_id, required, alternatives, forbidden) in [
        (
            "q-upd-diet-recipe",
            &["upd-diet-new", "pt-life-alergia-amendoim"][..],
            &[][..],
            &["upd-diet-old"][..],
        ),
        (
            "q-en-work-release-days",
            &["en-work-no-friday-deploys"][..],
            &["en-work-deploy-window", "dup-deploy-window"][..],
            &[][..],
        ),
    ] {
        let query = corpus["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == query_id)
            .unwrap()["text"]
            .as_str()
            .unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        let selected = select(hits, &evidence, &HashSet::new(), 6, query);
        let store_ids: HashSet<_> = selected.iter().map(|(m, _)| m.id.as_str()).collect();
        let response = hook(
            &deps,
            query_id,
            json!([{"role":"user","content":query}]),
            context(query_id, "direct", query),
        )
        .await;
        let actual = ids(&response);
        let hook_ids: HashSet<_> = actual
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        println!(
            "{}",
            json!({"diagnostic":"integrated-constraint-factual", "query_id":query_id,"store_selected":store_ids,"hook_selected":hook_ids,"required":required,"equivalent_alternatives":alternatives,"forbidden":forbidden})
        );
        for selected in [&store_ids, &hook_ids] {
            for id in required {
                assert!(
                    selected.contains(id),
                    "{query_id}: missing {id}; {selected:?}"
                );
            }
            for id in forbidden {
                assert!(!selected.contains(id), "{query_id}: forbidden {id}");
            }
            if !alternatives.is_empty() {
                assert_eq!(
                    alternatives
                        .iter()
                        .filter(|id| selected.contains(*id))
                        .count(),
                    1,
                    "{query_id}: {selected:?}"
                );
            }
            assert!(
                selected
                    .iter()
                    .all(|id| required.contains(id) || alternatives.contains(id)),
                "{query_id}: unrelated selection {selected:?}"
            );
        }
        rules_preserved(&response);
    }
    assert_eq!(
        std::fs::read(dir.path().join("hook-relevance/memories.jsonl")).unwrap(),
        before
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn governed_constraints_original_facts_all_lanes_and_provider_counts() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let mut failures = Vec::new();
    let mut counts = Vec::new();
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
        let dir = tempfile::tempdir().unwrap();
        let cache: Option<Value> = raw.map(|r| serde_json::from_str(r).unwrap());
        let boundary = if let Some(cache) = &cache {
            Some(super::super::cached_boundary::boundary(dir.path(), cache.clone()).await)
        } else {
            None
        };
        let deps = boundary
            .as_ref()
            .map(|b| b.deps.clone())
            .unwrap_or_else(|| offline(dir.path()));
        let store = deps.store().await;
        let bank_name = if cache.is_some() {
            "cached-main"
        } else {
            "hook-relevance"
        };
        let (bank, _) = store.ensure_bank(bank_name, None).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        for fact in corpus["memories"].as_array().unwrap() {
            let id = fact["key"].as_str().unwrap();
            let text = fact["text"].as_str().unwrap();
            let mut m = record(id, text);
            let at = now_ms().saturating_sub(fact["age_days"].as_u64().unwrap() * 86_400_000);
            m.created_at = at;
            m.updated_at = at;
            m.stated_at = Some(at);
            m.entities = serde_json::from_value(fact["entities"].clone()).unwrap();
            m.tags = serde_json::from_value(fact["tags"].clone()).unwrap();
            m.pinned = fact["pinned"].as_bool().unwrap_or(false);
            m.corroboration = fact["corroboration"].as_u64().unwrap_or(0) as u32;
            bank.commit(m).await.unwrap();
            if let Some(cache) = &cache {
                assert_eq!(cache["items"][id]["text"], text);
                let vector: Vec<f32> =
                    serde_json::from_value(cache["items"][id]["v"].clone()).unwrap();
                assert_eq!(vector.len(), cache["dims"].as_u64().unwrap() as usize);
                bank.set_vector(id, cache["model"].as_str().unwrap(), text, vector)
                    .await
                    .unwrap();
            }
        }
        let before = std::fs::read(dir.path().join(bank_name).join("memories.jsonl")).unwrap();
        let mut executed = 0;
        for (query_id, required, equivalents) in [
            (
                "q-en-work-late-week-rollout",
                &["en-work-no-friday-deploys"][..],
                &["en-work-deploy-window", "dup-deploy-window"][..],
            ),
            (
                "q-en-work-release-days",
                &["en-work-no-friday-deploys"][..],
                &["en-work-deploy-window", "dup-deploy-window"][..],
            ),
            (
                "q-upd-diet-recipe",
                &["upd-diet-new", "pt-life-alergia-amendoim"][..],
                &[][..],
            ),
            (
                "q-pt-life-lex-cafe-v60",
                &[][..],
                &["pt-life-cafe-v60", "dup-cafe-v60"][..],
            ),
        ] {
            let original = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap();
            let query = original["text"].as_str().unwrap();
            for id in required.iter().chain(equivalents.iter()) {
                assert!(original["expect"].as_array().unwrap().contains(&json!(id)));
            }
            let (qv, model) = if let Some(cache) = &cache {
                assert_eq!(cache["items"][query_id]["text"], query);
                (
                    Some(
                        serde_json::from_value::<Vec<f32>>(cache["items"][query_id]["v"].clone())
                            .unwrap(),
                    ),
                    cache["model"].as_str().unwrap(),
                )
            } else {
                (None, "")
            };
            let (ranked, evidence) = bank
                .recall_evidence(query, qv.as_deref(), model, usize::MAX, 30, false, 0.0)
                .await;
            let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
            let store_ids: HashSet<_> = selected.iter().map(|(m, _)| m.id.as_str()).collect();
            let session = format!("governed-{lane}-{query_id}");
            let response = if cache.is_some() {
                let request: PreGenerateInput = serde_json::from_value(json!({
                    "session_id":session,"turn_id":format!("t-{session}"),"step":0,
                    "metadata":{"memory_bank":bank_name},
                    "recall_context":{"version":1,"session_id":session,"turn_id":format!("t-{session}"),"source":{"kind":"direct","entry_id":"e1"},"query":{"entry_id":"e1","text":query}},
                    "generate":{"system_prompt":"offline","messages":[{"role":"user","content":query}]}
                })).unwrap();
                serde_json::to_value(pre_generate(&deps, request).await.unwrap()).unwrap()
            } else {
                hook(
                    &deps,
                    &session,
                    json!([{"role":"user","content":query}]),
                    context(&session, "direct", query),
                )
                .await
            };
            let actual = ids(&response);
            let hook_ids: HashSet<_> = actual
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect();
            for (consumer, selected) in [("store", &store_ids), ("hook", &hook_ids)] {
                let missing: Vec<_> = required
                    .iter()
                    .filter(|id| !selected.contains(**id))
                    .collect();
                let equivalent_count = equivalents
                    .iter()
                    .filter(|id| selected.contains(**id))
                    .count();
                let noise: Vec<_> = selected
                    .iter()
                    .filter(|id| !required.contains(id) && !equivalents.contains(id))
                    .collect();
                println!(
                    "{}",
                    json!({"diagnostic":"governed-original-factual","lane":lane,"model":model,"query_id":query_id,"consumer":consumer,"selected":selected,"missing":missing,"noise":noise,"equivalent_count":equivalent_count,"required":required,"equivalents":equivalents,"provider_calls":0})
                );
                if !missing.is_empty()
                    || !noise.is_empty()
                    || (!equivalents.is_empty() && equivalent_count != 1)
                    || selected.len() != required.len() + usize::from(!equivalents.is_empty())
                {
                    failures.push(format!("{lane}/{query_id}/{consumer}: {selected:?}"));
                }
            }
            if !response["mutations"]["system_prompt"]
                .as_str()
                .unwrap_or("")
                .contains("Preserve the permanent safety rule.")
            {
                failures.push(format!("{lane}/{query_id}: rule lost"));
            }
            executed += 1;
        }
        if let Some(boundary) = &boundary {
            let calls = boundary.calls.lock().unwrap();
            if calls.len() != 4 || calls.iter().any(|c| c.starts_with("UNEXPECTED:")) {
                failures.push(format!("{lane}: unexpected calls {calls:?}"));
            }
        }
        assert_eq!(
            std::fs::read(dir.path().join(bank_name).join("memories.jsonl")).unwrap(),
            before
        );
        counts.push(json!({"lane":lane,"model":cache.as_ref().map(|c| &c["model"]),"executed_controls":executed}));
    }
    println!(
        "{}",
        json!({"diagnostic":"governed-all-lane-counts","counts":counts})
    );
    assert_eq!(counts.len(), 3);
    assert!(counts.iter().all(|c| c["executed_controls"] == 4));
    assert!(
        failures.is_empty(),
        "original factual constraints: {failures:?}"
    );
}

fn record(id: &str, text: &str) -> Memory {
    Memory {
        id: id.into(),
        text: text.into(),
        entities: Vec::new(),
        tags: Vec::new(),
        confidence: crate::types::Confidence::Stated,
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

#[tokio::test]
async fn personal_ingestible_constraints_have_store_and_product_counterparts() {
    let mut failures = Vec::new();
    let mut executed = 0;
    let positives = [
        // Original factual positive: the source asserts the allergy, while the
        // finite language taxonomy (not a fabricated source field) types its argument.
        "User has a food allergy to sesame.",
        "A usuária tem alergia grave a castanha e leva medicação consigo.",
        // New, independently witnessed neutral ingestible controls.
        "User has a food allergy to sesame after eating it.",
        "User has a food allergy to marula after drinking it.",
        "A usuária tem alergia grave a castanha e leva medicação antialérgica consigo.",
        "A usuária tem alergia grave a painço após ingerir isso.",
    ];
    let negatives = [
        "The visitor has a food allergy to sesame.",
        "User's colleague has a food allergy to sesame.",
        "User reports that a visitor has a food allergy to sesame.",
        "User does not have a food allergy to sesame.",
        "If user has a food allergy to sesame, notify the visitor.",
        "User quotes: \"I have a food allergy to sesame.\"",
        "User documents a food allergy to sesame in a fictional recipe.",
        "User has a food allergy to sesame in a fictional recipe.",
        "User has a food allergy to sesame for the visitor.",
        "Sesame has a food allergy to user.",
        "A visitante tem alergia grave a castanha.",
        "A usuária relata que a visitante tem alergia grave a castanha.",
        "A usuária não tem alergia grave a castanha.",
        "User is allergic to verbose configuration files.",
        "User has a food allergy to verbose configuration files.",
        "User has a food allergy to unfamiliar filing conventions.",
        "A usuária tem alergia grave a castanha.",
        "User has a food allergy to sesame after not eating it.",
        "User has a food allergy to sesame if eating it.",
        "The visitor has a food allergy to sesame after eating it.",
        "User does not have a food allergy to sesame after eating it.",
        "If user has a food allergy to sesame after eating it, notify the visitor.",
        "User has a food allergy to sesame and does not carry allergy medication.",
        "User prefers sesame in meals.",
        "User's colleague prefers sesame in meals.",
        "User has a food allergy to sesame server.",
        "User has a food allergy to sesame library after eating it.",
        "User has a food allergy to pear configuration files.",
        "User has a food allergy to almond protocol.",
        "User has a food allergy to rice code.",
        "User has a food allergy to milk interface.",
        "User has a food allergy to unknownnominal.",
        "User has a food allergy to marula.",
        "User has a food allergy to sesame or milk.",
        "User has a food allergy to sesame and milk.",
        "User has a food allergy to \"sesame\".",
        "User has a food allergy to alleged sesame.",
        "User has a food allergy to sesame-flavored software.",
        "A usuária tem alergia grave a arquivos e leva medicação consigo.",
        "A usuária tem alergia grave a protocolo de castanha e leva medicação consigo.",
        "User has a food allergy to sesame oil server after eating it.",
        "A usuária prefere castanha nas refeições.",
    ];
    for query in [
        "Can you suggest a dinner recipe that fits my current diet?",
        "Recommend a meal for me?",
        "Você pode sugerir uma receita para mim?",
    ] {
        for (index, (text, expected)) in positives
            .iter()
            .map(|t| (*t, true))
            .chain(negatives.iter().map(|t| (*t, false)))
            .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let mut candidate = record("constraint", text);
            candidate.pinned = true;
            candidate.source = Some(crate::types::Provenance {
                session_id: Some("source-session".into()),
                entry_id: Some("source-entry".into()),
                agent: None,
            });
            bank.commit(candidate.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (hits, evidence) = bank
                .recall_evidence(query, None, "", 40, 30, false, 0.0)
                .await;
            let public = bank
                .recall_scaled(query, None, "", 40, 30, false, 0.0)
                .await;
            if query.starts_with("Can you") && text.starts_with("A usuária") {
                assert!(public.is_empty(), "the public lexical ranking is unchanged");
            }
            let proof = evidence
                .get("constraint")
                .and_then(|e| e.dietary_constraint.as_ref());
            if proof.is_some() != expected {
                failures.push(format!(
                    "proof: {query} / {text}; expected={expected}, actual={}",
                    proof.is_some()
                ));
            }
            let selected = select(hits.clone(), &evidence, &HashSet::new(), 10, query);
            if selected.iter().any(|(m, _)| m.id == "constraint") != expected {
                failures.push(format!("Store: {query} / {text}; expected={expected}"));
            }
            if admission_reasons(&hits, &evidence)
                .get("constraint")
                .is_some_and(|r| r.supported())
                != expected
            {
                failures.push(format!(
                    "pre-select support mismatch: {query} / {text}; expected={expected}"
                ));
            }
            let session = format!("constraint-{index}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            let expected_ids = if expected {
                json!(["constraint"])
            } else {
                json!([])
            };
            if ids(&response) != expected_ids {
                failures.push(format!(
                    "hook: {query} / {text}; expected={expected_ids}, actual={}",
                    ids(&response)
                ));
            }
            executed += 1;
            println!(
                "{}",
                json!({"diagnostic":"bodily-type-witness","query":query,"text":text,"expected":expected,"proof":proof,"store_selected":selected.iter().map(|(m,_)| &m.id).collect::<Vec<_>>(),"hook_selected":ids(&response)})
            );
            rules_preserved(&response);
            assert_eq!(
                bank.get("constraint").await.unwrap(),
                candidate,
                "recall cannot mutate pins, provenance or history"
            );
        }
    }
    println!(
        "{}",
        json!({"diagnostic":"bodily-type-witness-counts","executed_controls":executed,"expected_controls":3 * (positives.len() + negatives.len())})
    );
    assert_eq!(executed, 3 * (positives.len() + negatives.len()));
    assert!(failures.is_empty(), "bodily type witnesses: {failures:?}");
}

// Heldout from the original factual corpus: ordinary foods across several
// classes, and compositional technical negatives in the exact same frame.
// No test ID or expected value is visible to the product taxonomy.
#[tokio::test]
async fn finite_food_taxonomy_whole_arguments_have_preselect_and_hook_witnesses() {
    let query = "Recommend a meal for me?";
    let mut executed = 0;
    for argument in [
        "hazelnut",
        "barley",
        "lentils",
        "peach",
        "shrimp",
        "sunflower oil",
        "farinha de milho",
    ] {
        for (text, expected) in [
            (format!("User has a food allergy to {argument}."), true),
            (
                format!("User has a food allergy to {argument} server."),
                false,
            ),
            (
                format!("User has a food allergy to {argument} configuration files."),
                false,
            ),
            (
                format!("User has a food allergy to {argument} protocol."),
                false,
            ),
            (
                format!("The visitor has a food allergy to {argument}."),
                false,
            ),
            (
                format!("User does not have a food allergy to {argument}."),
                false,
            ),
            (
                format!("If user has a food allergy to {argument}, notify me."),
                false,
            ),
            (
                format!("User quotes: I have a food allergy to {argument}."),
                false,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = record("material", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (hits, evidence) = bank
                .recall_evidence(query, None, "", 40, 30, false, 0.0)
                .await;
            let proof = evidence
                .get("material")
                .and_then(|e| e.dietary_constraint.as_ref());
            assert_eq!(proof.is_some(), expected, "pre-select: {text}");
            let reasons = admission_reasons(&hits, &evidence);
            assert_eq!(
                reasons.get("material").is_some_and(|r| r.supported()),
                expected,
                "support: {text}"
            );
            if let Some(proof) = proof {
                assert_eq!(
                    proof["assertion_source"],
                    "candidate_subject_predicate_argument"
                );
                assert_eq!(proof["type_witness"], "lexical_food_material_type");
                assert!(
                    proof["type_witness_bytes"].is_null(),
                    "no ingestion assertion in source"
                );
                assert_eq!(
                    proof["lexical_type_assumption"]["taxonomy"],
                    "bounded_food_material_v1"
                );
                assert_eq!(
                    proof["lexical_type_assumption"]["source"],
                    "general_language_type_knowledge_not_candidate_assertion"
                );
                let range = proof["substance_bytes"].as_array().unwrap();
                assert_eq!(
                    &text.to_lowercase()
                        [range[0].as_u64().unwrap() as usize..range[1].as_u64().unwrap() as usize],
                    argument
                );
            }
            let selected = select(hits, &evidence, &HashSet::new(), 10, query);
            assert_eq!(selected.len(), usize::from(expected), "Store: {text}");
            let response = hook(
                &deps,
                "material",
                json!([{"role":"user","content":query}]),
                context("material", "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["material"])
                } else {
                    json!([])
                },
                "hook: {text}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("material").await.unwrap(), original);
            executed += 1;
        }
    }
    assert_eq!(executed, 56);
    println!(
        "{}",
        json!({"diagnostic":"finite-food-taxonomy-heldout", "executed_controls":executed,"positive_controls":7,"negative_controls":49,"lane":"lexical","new_vectors":false})
    );
}

#[tokio::test]
async fn bodily_constraint_does_not_authorize_non_ingestible_or_foreign_requests() {
    for query in [
        "Recommend a recipe for the visitor?",
        "Can you suggest a configuration recipe that fits my current diet?",
        "How do I serialize food allergy recipe records?",
        "Does the visitor suggest a meal for me?",
        "User quotes: recommend a meal for me?",
        "Can you suggest a dinner recipe that fits my colleague's current diet?",
        "Recommend a meal for me without allergy handling?",
        "If I want a meal, recommend a recipe for me?",
        "User quotes: \"Recommend a meal for me?\"",
        "Can you suggest a dinner recipe that fits my visitor's current diet?",
        "Recommend a software recipe for me?",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(record(
            "restriction",
            "A usuária tem alergia grave a castanha.",
        ))
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", 40, 30, false, 0.0)
            .await;
        assert!(
            evidence.values().all(|e| e.dietary_constraint.is_none()),
            "{query}"
        );
        assert!(
            select(hits, &evidence, &HashSet::new(), 10, query).is_empty(),
            "{query}"
        );
        let response = hook(
            &deps,
            "foreign-request",
            json!([{"role":"user","content":query}]),
            context("foreign-request", "direct", query),
        )
        .await;
        assert_eq!(ids(&response), json!([]), "{query}");
        rules_preserved(&response);
    }
}

#[tokio::test]
async fn filtering_equivalence_keeps_every_argument_and_persistent_record() {
    let a =
        "A usuária prepara infusão coada no Filtro Azul toda tarde, com folhas de origem serrana.";
    let b = "Toda tarde a usuária passa infusão no Filtro Azul usando folhas de origem serrana.";
    assert!(crate::extract::same_fact(a, b));
    assert!(crate::extract::same_fact(b, a));
    for other in [
        "Toda noite a usuária passa infusão no Filtro Azul usando folhas de origem serrana.",
        "Toda tarde a usuária passa infusão no Filtro Verde usando folhas de origem serrana.",
        "Toda tarde a usuária passa infusão no Filtro Azul usando folhas de origem serrana e mel.",
        "Toda tarde a usuária passa folhas no Filtro Azul usando infusão de origem serrana.",
        "Toda tarde a visitante passa infusão no Filtro Azul usando folhas de origem serrana.",
        "Toda tarde a usuária não passa infusão no Filtro Azul usando folhas de origem serrana.",
    ] {
        assert!(!crate::extract::same_fact(a, other), "{other}");
        assert!(!crate::extract::same_fact(other, a), "{other}");
    }
    for pinned in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        let first = record("first", a);
        let mut second = record("second", b);
        second.pinned = pinned;
        second.corroboration = 4;
        bank.commit(first.clone()).await.unwrap();
        bank.commit(second.clone()).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let query = "Infusão no Filtro Azul com folhas de origem serrana?";
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", 40, 30, false, 0.0)
            .await;
        let selected = select(hits, &evidence, &HashSet::new(), 10, query);
        assert_eq!(selected.len(), 1);
        if pinned {
            assert_eq!(selected[0].0.id, "second");
        }
        let response = hook(
            &deps,
            "filtering",
            json!([{"role":"user","content":query}]),
            context("filtering", "direct", query),
        )
        .await;
        assert_eq!(ids(&response).as_array().unwrap().len(), 1);
        if pinned {
            assert_eq!(ids(&response), json!(["second"]));
        }
        rules_preserved(&response);
        assert_eq!(bank.get("first").await.unwrap(), first);
        assert_eq!(bank.get("second").await.unwrap(), second);
    }
}
