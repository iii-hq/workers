//! Technical facts are required independently of the admission policy.
//! Synthetic controls are lexical; original cached lanes use exact existing
//! query AND candidate vectors, never transplanted or generated embeddings.
use super::*;

#[path = "requested_attribute.rs"]
mod requested_attribute;

fn attested(id: &str, text: &str) -> Memory {
    let mut memory = record(id, text);
    memory.confidence = crate::types::Confidence::Extracted;
    memory.source = Some(crate::types::Provenance {
        session_id: Some("synthetic-technical-source".into()),
        entry_id: Some(format!("assertion-{id}")),
        ..Default::default()
    });
    memory
}

#[tokio::test]
async fn technical_relations_minimal_pairs_store_public_and_hook() {
    let cases = [
        (
            "When I run integration tests for storage, do I use real engines?",
            "User executes integration tests against real MariaDB using Podman.",
            "Integration tests execute user against real MariaDB using Podman.",
        ),
        (
            "Before I repair a reported defect in code, what do I do first?",
            "User creates a failing regression test that reproduces a reported defect before modifying the implementation.",
            "User modifies the implementation before creating a failing regression test that reproduces a reported defect.",
        ),
        (
            "Which JVM language and framework do I normally use for microservices?",
            "User implements most of their microservices in Scala with Play.",
            "Scala implements most of their microservices in user with Play.",
        ),
        (
            "Which protocol do I use for private service communication?",
            "User uses AMQP for private microservices.",
            "AMQP uses user for private microservices.",
        ),
        (
            "How many lines should my patch have before review?",
            "User limits review patches to 175 changed lines so a reviewer can finish them in one sitting.",
            "A reviewer limits user to 175 changed lines so review patches can finish them in one sitting.",
        ),
    ];
    let mut executed = 0;
    let mut request_controls = 0;
    let mut failures = Vec::new();
    for (query, positive, inversion) in cases {
        let variants = [
            (positive.to_owned(), true),
            (positive.replacen("User ", "User's colleague ", 1), false),
            (positive.replacen("User ", "User never ", 1), false),
            (format!("User quotes: \"{positive}\""), false),
            (format!("If {positive} ask the reviewer."), false),
            (inversion.to_owned(), false),
        ];
        for (text, expected) in variants {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = attested("practice", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let public = bank.recall(query, None, "", 30, 30, false).await;
            if expected && !public.iter().any(|(m, _)| m.id == "practice") {
                failures.push(format!("public: {query} / {text}"));
            }
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let proved = evidence
                .get("practice")
                .and_then(|e| e.technical_practice.as_ref())
                .is_some();
            if proved != expected {
                failures.push(format!("proof: {query} / {text}: {proved} != {expected}"));
            }
            let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
            if selected.len() != usize::from(expected) {
                failures.push(format!("selection: {query} / {text}: {selected:?}"));
            }
            let response = hook(
                &deps,
                "technical-pair",
                json!([{"role":"user","content":query}]),
                context("technical-pair", "direct", query),
            )
            .await;
            let expected_ids = if expected {
                json!(["practice"])
            } else {
                json!([])
            };
            if ids(&response) != expected_ids {
                failures.push(format!(
                    "hook: {query} / {text}: {} != {expected_ids}",
                    ids(&response)
                ));
            }
            rules_preserved(&response);
            assert_eq!(bank.get("practice").await.unwrap(), original);
            executed += 1;
        }
        // Owner, modality and assertion status apply to requests as well.
        for bad_query in [
            query
                .replace(" I ", " the visitor ")
                .replace(" my ", " the visitor's "),
            format!("If {query}"),
            query
                .replace(" I ", " I never ")
                .replace(" my ", " not my "),
            format!("User quotes: \"{query}\""),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            bank.commit(attested("practice", positive)).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (hits, evidence) = bank
                .recall_evidence(&bad_query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            if evidence.values().any(|e| e.technical_practice.is_some()) {
                failures.push(format!("request proof: {bad_query}"));
            }
            if !select(hits, &evidence, &HashSet::new(), 6, &bad_query).is_empty() {
                failures.push(format!("request selection: {bad_query}"));
            }
            let response = hook(
                &deps,
                "technical-query",
                json!([{"role":"user","content":bad_query}]),
                context("technical-query", "direct", &bad_query),
            )
            .await;
            if ids(&response) != json!([]) {
                failures.push(format!("request hook: {bad_query}: {}", ids(&response)));
            }
            rules_preserved(&response);
            request_controls += 1;
        }
    }
    assert_eq!(executed, 30);
    assert_eq!(request_controls, 20);
    assert!(
        failures.is_empty(),
        "all minimal pairs and request controls: {failures:?}"
    );
}

#[tokio::test]
async fn technical_relations_same_words_wrong_domain_and_unrelated_default() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for (query, positive, negative) in [
        (
            "When I run integration tests for storage, do I use real engines?",
            "User runs integration tests against real Oracle using Docker.",
            "User runs mocked integration tests instead of tests against real Oracle using Docker.",
        ),
        (
            "Which protocol do I use for private service communication?",
            "User prefers HTTP with JSON between internal services and AMQP for public endpoints.",
            // The mixed-domain AMQP assertion remains a supported control in
            // requested_attribute::communication_coordinated_domain_exact_argument_and_hook.
            "User prefers HTTP with JSON between public endpoints.",
        ),
        (
            "Which JVM language and framework do I normally use for microservices?",
            "User implements most of their microservices in Java with Quarkus.",
            "User implements most of their microservices in Python with Django.",
        ),
        (
            "Which JVM language and framework do I normally use for microservices?",
            "User implements most of their microservices in Java with Quarkus.",
            "User implements most of their command line tools in Java with Quarkus.",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(attested("positive", positive)).await.unwrap();
        bank.commit(attested("negative", negative)).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        if evidence
            .get("positive")
            .is_none_or(|e| e.technical_practice.is_none())
        {
            failures.push(format!("positive proof: {query} / {positive}"));
        }
        if evidence
            .get("negative")
            .is_some_and(|e| e.technical_practice.is_some())
        {
            failures.push(format!("negative proof: {query} / {negative}"));
        }
        let selected = select(hits, &evidence, &HashSet::new(), 6, query);
        if selected
            .iter()
            .map(|(m, _)| m.id.as_str())
            .collect::<Vec<_>>()
            != ["positive"]
        {
            failures.push(format!("domain selection: {query}: {selected:?}"));
        }
        let response = hook(
            &deps,
            "technical-domain",
            json!([{"role":"user","content":query}]),
            context("technical-domain", "direct", query),
        )
        .await;
        if ids(&response) != json!(["positive"]) {
            failures.push(format!("domain hook: {query}: {}", ids(&response)));
        }
        rules_preserved(&response);
        executed += 1;
    }
    // A beverage preference and ordinary factual question never enter the
    // technical grammar, even though the question also requests a default.
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.commit(attested("beverage", "User prefers rooibos tea."))
        .await
        .unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let query = "Which tea do I prefer by default?";
    let (hits, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    assert!(evidence.values().all(|e| !e.technical_scope_required));
    assert_eq!(select(hits, &evidence, &HashSet::new(), 6, query).len(), 1);
    let response = hook(
        &deps,
        "unrelated-preference",
        json!([{"role":"user","content":query}]),
        context("unrelated-preference", "direct", query),
    )
    .await;
    assert_eq!(ids(&response), json!(["beverage"]));
    rules_preserved(&response);
    assert_eq!(executed, 4);
    assert!(failures.is_empty(), "all domain controls: {failures:?}");
}

#[tokio::test]
async fn technical_relations_target_subject_is_not_a_personal_practice_request() {
    for (query, positive, inverse) in [
        (
            "Which regression test reproduces the defect before a repair?",
            "User writes a regression test that reproduces the defect before any repair.",
            "User writes a defect that reproduces the regression test before any repair.",
        ),
        (
            "Which integration test reproduces the storage defect before a code repair?",
            "User writes an integration test that reproduces the storage defect before a code repair.",
            "User writes a storage defect that reproduces the integration test before a code repair.",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for (id, text) in [("target", positive), ("inverse", inverse)] {
            bank.commit(attested(id, text)).await.unwrap();
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.").unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(evidence.values().all(|e| !e.technical_scope_required));
        assert!(evidence["target"].target_scope_required);
        assert!(evidence["target"].target_scope_matched);
        assert!(evidence.get("inverse").is_none_or(|e| !e.target_scope_matched));
        let selected = select(hits, &evidence, &HashSet::new(), 6, query);
        assert_eq!(selected.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(), ["target"]);
        let response = hook(
            &deps,
            "technical-target",
            json!([{"role":"user","content":query}]),
            context("technical-target", "direct", query),
        ).await;
        assert_eq!(ids(&response), json!(["target"]), "{query}");
        rules_preserved(&response);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn technical_relations_original_facts_all_cached_lanes() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let required = [
        ("q-en-work-storage-checks", "en-work-testcontainers"),
        ("q-en-work-crash-ticket", "en-work-regression-test-first"),
        ("q-en-work-default-stack", "en-work-kotlin-spring"),
        ("q-en-work-wire-protocol", "en-work-grpc-internal"),
        ("q-en-work-branch-size", "en-work-small-prs"),
    ];
    let mut failures = Vec::new();
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
        let cache: Option<Value> = raw.map(|r| serde_json::from_str(r).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let boundary = if let Some(cache) = &cache {
            Some(super::super::super::cached_boundary::boundary(dir.path(), cache.clone()).await)
        } else {
            None
        };
        let deps = boundary
            .as_ref()
            .map(|b| b.deps.clone())
            .unwrap_or_else(|| offline(dir.path()));
        let bank_name = if cache.is_some() {
            "cached-main"
        } else {
            "hook-relevance"
        };
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank(bank_name, None).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        for fact in corpus["memories"].as_array().unwrap() {
            let id = fact["key"].as_str().unwrap();
            let text = fact["text"].as_str().unwrap();
            let mut memory = attested(id, text);
            memory.entities = serde_json::from_value(fact["entities"].clone()).unwrap();
            memory.tags = serde_json::from_value(fact["tags"].clone()).unwrap();
            let at = now_ms().saturating_sub(fact["age_days"].as_u64().unwrap() * 86_400_000);
            memory.created_at = at;
            memory.updated_at = at;
            memory.stated_at = Some(at);
            memory.pinned = fact["pinned"].as_bool().unwrap_or(false);
            memory.corroboration = fact["corroboration"].as_u64().unwrap_or(0) as u32;
            bank.commit(memory).await.unwrap();
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
        for (query_id, expected) in required {
            let query = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap()["text"]
                .as_str()
                .unwrap();
            let vector: Option<Vec<f32>> = cache.as_ref().map(|c| {
                assert_eq!(c["items"][query_id]["text"], query);
                serde_json::from_value(c["items"][query_id]["v"].clone()).unwrap()
            });
            let model = cache
                .as_ref()
                .and_then(|c| c["model"].as_str())
                .unwrap_or("");
            let public = bank
                .recall(query, vector.as_deref(), model, usize::MAX, 30, false)
                .await;
            assert!(
                public.iter().any(|(m, _)| m.id == expected),
                "{lane}/{query_id}: public factual candidate missing"
            );
            let (hits, evidence) = bank
                .recall_evidence(query, vector.as_deref(), model, usize::MAX, 30, false, 0.0)
                .await;
            assert!(
                evidence[expected].technical_practice.is_some(),
                "{lane}/{query_id}: missing witness"
            );
            let selected = select(hits, &evidence, &HashSet::new(), 6, query);
            let selected_ids: Vec<_> = selected.iter().map(|(m, _)| m.id.as_str()).collect();
            let session = format!("technical-{lane}-{query_id}");
            let request: PreGenerateInput = serde_json::from_value(json!({
                "session_id":session,"turn_id":format!("turn-{session}"),"step":0,
                "metadata":{"memory_bank":bank_name},
                "recall_context":context(&session,"direct",query),
                "generate":{"system_prompt":"offline","messages":[{"role":"user","content":query}]}
            }))
            .unwrap();
            let response =
                serde_json::to_value(pre_generate(&deps, request).await.unwrap()).unwrap();
            let hook_ids = ids(&response);
            println!(
                "{}",
                json!({"diagnostic":"technical-original-factual","lane":lane,"query_id":query_id,"expected":[expected],"selected":selected_ids,"hook":hook_ids,"proof":evidence[expected].technical_practice,"inference_calls":0})
            );
            if selected_ids != [expected] || hook_ids != json!([expected]) {
                failures.push(format!(
                    "{lane}/{query_id}: store={selected_ids:?} hook={hook_ids}"
                ));
            }
            rules_preserved(&response);
        }
        if let Some(boundary) = &boundary {
            let calls = boundary.calls.lock().unwrap();
            assert_eq!(calls.len(), 5, "{lane}: {calls:?}");
            assert!(!calls.iter().any(|c| c.starts_with("UNEXPECTED:")));
        }
        assert_eq!(
            std::fs::read(dir.path().join(bank_name).join("memories.jsonl")).unwrap(),
            before
        );
    }
    assert!(
        failures.is_empty(),
        "original factual residual: {failures:?}"
    );
}

#[tokio::test]
async fn technical_relations_unattested_extraction_never_borrows_lexical_support() {
    let query = "Which JVM language and web stack do I default to when I start a new microservice?";
    let text =
        "User writes most of her backend services in Kotlin with Spring Boot 3 and coroutines.";
    for (confidence, source, expected) in [
        (crate::types::Confidence::Extracted, None, false),
        (
            crate::types::Confidence::Extracted,
            Some(crate::types::Provenance {
                session_id: Some("synthetic-source".into()),
                entry_id: None,
                ..Default::default()
            }),
            false,
        ),
        (
            crate::types::Confidence::Extracted,
            Some(crate::types::Provenance {
                session_id: Some("synthetic-source".into()),
                entry_id: Some(" ".into()),
                ..Default::default()
            }),
            false,
        ),
        (
            crate::types::Confidence::Inferred,
            attested("p", text).source,
            false,
        ),
        (
            crate::types::Confidence::Extracted,
            attested("p", text).source,
            true,
        ),
        (crate::types::Confidence::Stated, None, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        let mut memory = record("practice", text);
        memory.confidence = confidence;
        memory.source = source;
        bank.commit(memory).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert_eq!(
            select(hits, &evidence, &HashSet::new(), 6, query).len(),
            usize::from(expected)
        );
        let response = hook(
            &deps,
            "technical-source",
            json!([{"role":"user","content":query}]),
            context("technical-source", "direct", query),
        )
        .await;
        assert_eq!(
            ids(&response),
            if expected {
                json!(["practice"])
            } else {
                json!([])
            }
        );
        rules_preserved(&response);
    }
}
