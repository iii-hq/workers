//! Finite activity/purpose controls through real Store and pre_generate.

#[tokio::test]
async fn life_reservation_object_domain_is_governed_not_incidental() {
    let text = "User prefers staying in family inns to large resorts.";
    for (n, (query, expected)) in [
        (
            "Pra minha próxima viagem à serra, reservo uma pousada ou algo mais familiar?",
            true,
        ),
        (
            "Pra minha próxima ida ao hotel, reservo um servidor ou algo mais potente?",
            false,
        ),
        (
            "Pra minha próxima viagem ao resort, reservo uma bicicleta ou uma van?",
            false,
        ),
        (
            "Pra minha próxima ida ao escritório, reservo um hotel ou uma pousada?",
            true,
        ),
        (
            "Pra minha próxima viagem à pousada, reservo uma sala ou algo mais amplo?",
            false,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let proof = life_evidence(query, text);
        assert_eq!(
            proof.as_ref().is_some_and(|p| p["matched"] == true),
            expected,
            "{query}: {proof:?}"
        );
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(fact("preference", text)).await.unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert_eq!(
            select(hits, &evidence, &HashSet::new(), 6, query).len(),
            usize::from(expected),
            "select {query}"
        );
        let session = format!("life-domain-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(
            ids(&response),
            if expected {
                json!(["preference"])
            } else {
                json!([])
            },
            "hook {query}"
        );
    }
}

use super::identity_relations::fact;
use super::*;
use crate::store::query_projection::life_evidence;

#[tokio::test]
async fn life_new_role_values_are_not_topics_or_inverted_arguments() {
    for (n, (query, text)) in [
        ("Which artists do I listen to while coding?", "User listens to jazz while programming, especially jazz and blues."),
        ("Which artists do I listen to while coding?", "User listens to jazz while programming, especially Jazz and Blues."),
        ("Which artists do I listen to while coding?", "Nina Simone listens to user while programming, especially Chet Baker."),
        ("Which artists do I listen to while coding?", "User listens to jazz while programming, especially Nina Simone says User never listens."),
        ("When do I video call my sister?", "User video calls with her sister Renata's visitor every wednesday morning to catch up."),
        ("When do I video call my sister?", "User video calls with her sister Renata not every wednesday morning to catch up."),
        ("When do I video call my sister?", "User video calls with her colleague Renata every wednesday morning to catch up."),
        ("When do I video call my sister?", "User video calls with her sister Renata every morning to catch up."),
        ("I have a headache, which medicine can I take safely?", "User is allergic to aspirin, so uses water for a headache."),
        ("I have a headache, which medicine can I take safely?", "Aspirin is allergic to user, so uses naproxen for a headache."),
    ].into_iter().enumerate() {
        assert_eq!(life_evidence(query,text).unwrap()["matched"],false,"{query}/{text}");
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance",None).await.unwrap();
        bank.commit(fact("near",text)).await.unwrap();
        let (hits,evidence) = bank.recall_evidence(query,None,"",usize::MAX,30,false,0.0).await;
        assert!(select(hits,&evidence,&HashSet::new(),6,query).is_empty(),"{query}/{text}");
        let session = format!("life-role-{n}");
        let response = hook(&deps,&session,json!([{"role":"user","content":query}]),context(&session,"direct",query)).await;
        assert_eq!(ids(&response),json!([]),"{query}/{text}");
    }
}

#[tokio::test]
async fn life_new_distinct_details_sources_pins_and_rules_survive() {
    for (n, (query, first, second)) in [
        (
            "Which artists do I listen to while coding?",
            "User listens to jazz while coding, especially Dinah Washington.",
            "User listens to soul while programming, especially Aretha Franklin.",
        ),
        (
            "I have a headache, which medication can I take safely?",
            "User is allergic to aspirin, so uses naproxen for a headache.",
            "User is allergic to ibuprofen, so uses acetaminophen for a migraine.",
        ),
        (
            "When do I video call my sister?",
            "User video calls with her sister Helena every monday evening to catch up.",
            "User video calls with her sister Helena every saturday morning to catch up.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let mut a = fact("first", first);
        let mut b = fact("second", second);
        a.pinned = true;
        a.source = Some(crate::types::Provenance {
            session_id: Some("first-evidence".into()),
            entry_id: Some("first".into()),
            agent: None,
        });
        b.source = Some(crate::types::Provenance {
            session_id: Some("second-evidence".into()),
            entry_id: Some("second".into()),
            agent: None,
        });
        bank.commit(a.clone()).await.unwrap();
        bank.commit(b.clone()).await.unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert_eq!(select(hits, &evidence, &HashSet::new(), 6, query).len(), 2);
        let session = format!("life-new-details-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response).as_array().unwrap().len(), 2);
        rules_preserved(&response);
        assert_eq!(bank.get("first").await.unwrap(), a);
        assert_eq!(bank.get("second").await.unwrap(), b);
    }
}

#[tokio::test]
async fn life_new_owned_roles_neutral_sources_and_contexts() {
    let cases = [
        (
            "Which artists do I listen to while coding?",
            "User listens to jazz while programming, especially Nina Simone and Chet Baker.",
            "User listens to jazz while cooking, especially Nina Simone and Chet Baker.",
        ),
        (
            "Quais cantoras eu ouço enquanto estou programando?",
            "Eu ouço samba enquanto programo, especialmente Dona Ivone Lara e Leci Brandão.",
            "Eu ouço samba enquanto programo.",
        ),
        (
            "I have a migraine, which medicine can I take safely?",
            "User is allergic to aspirin, so uses naproxen for a headache.",
            "User is allergic to aspirin, so uses naproxen for a fever.",
        ),
        (
            "Tenho dor de cabeça, qual medicamento eu posso tomar com segurança?",
            "Eu sou alérgica a ibuprofeno, então uso sumatriptana quando tenho enxaqueca.",
            "Eu sou alérgica a amendoim, então uso sumatriptana quando tenho enxaqueca.",
        ),
        (
            "When do I video call my sister?",
            "User video calls with her sister Renata every wednesday morning to catch up.",
            "User video calls with her brother Renato every wednesday morning to catch up.",
        ),
        (
            "Quando eu converso com minha mana por videochamada?",
            "Eu faço videochamada com minha irmã Beatriz toda sexta-feira de manhã para conversar.",
            "Eu faço telefonema com minha irmã Beatriz toda sexta-feira de manhã para conversar.",
        ),
    ];
    let mut count = 0;
    for (query, positive, near) in cases {
        let foreign = positive
            .replacen("User", "The visitor", 1)
            .replacen("Eu", "A visitante", 1);
        let negated = positive
            .replacen("User ", "User never ", 1)
            .replacen("Eu ", "Eu não ", 1);
        for (text, expected) in [
            (positive.to_owned(), true),
            (near.to_owned(), false),
            (foreign, false),
            (negated, false),
            (format!("User reports that {positive}"), false),
            (format!("User quotes: \"{positive}\""), false),
            (format!("If {positive}"), false),
            (format!("{positive} The visitor says otherwise."), false),
        ] {
            let proof = life_evidence(query, &text).unwrap();
            assert_eq!(proof["matched"], expected, "{query}/{text}: {proof}");
            assert_eq!(proof["certifies_medication_safety"], false);
            assert_eq!(proof["certifies_performer_nationality"], false);
            if expected {
                let norm = text.trim().to_lowercase();
                for key in ["owner_bytes", "governor_bytes", "value_bytes"] {
                    let span = proof[key].as_array().unwrap();
                    assert!(!norm
                        [span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize]
                        .is_empty());
                }
                if proof["kind"] == "recorded_medication_restriction" {
                    assert!(proof["restriction_bytes"].is_array());
                    assert!(proof["habit_governor_bytes"].is_array());
                    assert!(proof["complaint_bytes"].is_array());
                }
            }
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = fact("personal", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (hits, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            assert_eq!(
                admission_reasons(&hits, &evidence)
                    .get("personal")
                    .is_some_and(|r| r.supported()),
                expected,
                "preselect {query}/{text}"
            );
            assert_eq!(
                select(hits, &evidence, &HashSet::new(), 6, query).len(),
                usize::from(expected),
                "select {query}/{text}"
            );
            if expected {
                assert_eq!(
                    bank.recall(query, None, "", 1, 30, false).await[0].0.id,
                    "personal"
                );
            }
            let session = format!("life-new-{count}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["personal"])
                } else {
                    json!([])
                },
                "hook {query}/{text}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("personal").await.unwrap(), original);
            count += 1;
        }
    }
    assert_eq!(count, 48);
}

#[tokio::test]
async fn life_neutral_owned_purpose_preference_and_learning() {
    let cases = [
        (
            "What do I drink before bed to sleep better?",
            "User drinks linden tea to sleep better.",
            "User drinks linden tea after dinner.",
        ),
        (
            "O que eu devo beber antes de dormir?",
            "Eu tomo chá de erva doce para dormir melhor.",
            "Eu tomo chá de erva doce para estudar melhor.",
        ),
        (
            "Which lodging do I prefer?",
            "User prefers staying in family inns rather than large resorts when traveling.",
            "User booked a room in family inns rather than large resorts when traveling.",
        ),
        (
            "Como anda meu curso de cerâmica?",
            "Eu comecei aulas de olaria e estou aprendendo a fazer canecas no torno.",
            "Eu instalei um pacote de cerâmica e estou aprendendo a fazer canecas no torno.",
        ),
        (
            "How is my pottery course going?",
            "User started ceramics classes and is learning to shape vases on a wheel.",
            "User started ceramics classes and is learning to shape software on a wheel.",
        ),
    ];
    let mut count = 0;
    for (query, positive, domain_negative) in cases {
        let foreign = positive
            .replacen("User", "The visitor", 1)
            .replacen("Eu", "A visitante", 1);
        let negated = positive
            .replacen("User ", "User never ", 1)
            .replacen("Eu ", "Eu não ", 1);
        for (text, expected) in [
            (positive.to_owned(), true),
            (domain_negative.to_owned(), false),
            (foreign, false),
            (negated, false),
            (format!("User reports that {positive}"), false),
            (format!("User quotes: \"{positive}\""), false),
            (format!("If {positive} ask the visitor."), false),
            ("Linden tea drinks user to sleep better.".into(), false),
            ("User drinks unknown tea to sleep better.".into(), false),
        ] {
            let proof = life_evidence(query, &text).unwrap();
            assert_eq!(proof["matched"], expected, "{query} / {text}: {proof}");
            if expected {
                let norm = text.trim().to_lowercase();
                let span = proof["value_bytes"].as_array().unwrap();
                assert_eq!(
                    &norm[span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize],
                    proof["value"].as_str().unwrap()
                );
                assert!(proof["governor_bytes"].is_array());
                assert!(proof["owner_bytes"].is_array());
            }
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = fact("activity", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (hits, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            assert_eq!(
                admission_reasons(&hits, &evidence)
                    .get("activity")
                    .is_some_and(|r| r.supported()),
                expected,
                "preselect {query} / {text}"
            );
            assert_eq!(
                select(hits, &evidence, &HashSet::new(), 6, query).len(),
                usize::from(expected)
            );
            let session = format!("life-neutral-{count}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["activity"])
                } else {
                    json!([])
                },
                "hook {query} / {text}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("activity").await.unwrap(), original);
            count += 1;
        }
    }
    assert_eq!(count, 45);
}

#[test]
fn life_foreign_quoted_and_requested_attribute_queries_are_not_certified() {
    let text = "User prefers staying in small guesthouses to large chain hotels.";
    for query in [
        "Which hotel did I book?",
        "Which lodging does the visitor prefer?",
        "If I traveled, which lodging would I prefer?",
        "User quotes: Which lodging do I prefer?",
        "How many people sit on my lodging team?",
        "Which room is my pottery course in?",
        "How many partitions does my pottery publication have?",
    ] {
        assert!(life_evidence(query, text).is_none(), "{query}");
    }
}

#[tokio::test]
async fn life_requested_values_and_unattested_sources_never_inherit_activity_proof() {
    for (n, (query, text)) in [
        (
            "Which hotel did I book?",
            "User prefers staying in family inns to large resorts.",
        ),
        (
            "How many people sit on the astronomy team?",
            "User is a senior acoustics engineer on the astronomy team at Juniper Labs.",
        ),
        (
            "Which room is my pottery course in?",
            "User started pottery classes and is learning to throw mugs on a wheel.",
        ),
        (
            "How many partitions does the publication have?",
            "The publication describes pottery and three ceramic bowls.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        bank.commit(fact("near", text)).await.unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(
            admission_reasons(&hits, &evidence)
                .values()
                .all(|r| !r.supported()),
            "{query}"
        );
        assert!(select(hits, &evidence, &HashSet::new(), 6, query).is_empty());
        let session = format!("life-property-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), json!([]), "{query}");
        rules_preserved(&response);
    }
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    let mut m = fact("unattested", "User drinks linden tea to sleep better.");
    m.confidence = crate::types::Confidence::Extracted;
    bank.commit(m).await.unwrap();
    let query = "What do I drink before bed to sleep better?";
    let (hits, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    assert!(admission_reasons(&hits, &evidence)
        .values()
        .all(|r| !r.supported()));
    assert!(select(hits, &evidence, &HashSet::new(), 6, query).is_empty());
}

#[tokio::test]
async fn life_distinct_values_pins_sources_and_rules_remain_intact() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut a = fact("first", "User drinks linden tea to sleep better.");
    let mut b = fact("second", "User drinks rooibos tea to sleep better.");
    a.pinned = true;
    a.source = Some(crate::types::Provenance {
        session_id: Some("first-source".into()),
        entry_id: Some("a".into()),
        agent: None,
    });
    b.source = Some(crate::types::Provenance {
        session_id: Some("second-source".into()),
        entry_id: Some("b".into()),
        agent: None,
    });
    bank.commit(a.clone()).await.unwrap();
    bank.commit(b.clone()).await.unwrap();
    let query = "What do I drink before bed to sleep better?";
    let (hits, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    assert_eq!(select(hits, &evidence, &HashSet::new(), 6, query).len(), 2);
    let response = hook(
        &deps,
        "life-details",
        json!([{"role":"user","content":query}]),
        context("life-details", "direct", query),
    )
    .await;
    assert_eq!(ids(&response).as_array().unwrap().len(), 2);
    rules_preserved(&response);
    assert_eq!(bank.get("first").await.unwrap(), a);
    assert_eq!(bank.get("second").await.unwrap(), b);
}

#[tokio::test]
async fn life_typed_discovery_survives_crowded_banks_without_score_proof() {
    for (n, (query, text)) in [
        (
            "What do I drink before bed to sleep better?",
            "User drinks rooibos tea to sleep better.",
        ),
        (
            "Which lodging do I prefer?",
            "User prefers staying in family inns to large resorts.",
        ),
        (
            "How is my pottery course going?",
            "User started pottery classes and is learning to throw mugs on a wheel.",
        ),
        (
            "Which artists do I listen to while coding?",
            "User listens to blues while programming, especially Etta James and Otis Redding.",
        ),
        (
            "I have a headache, which medicine can I take without risk?",
            "User is allergic to ibuprofen, so uses acetaminophen for a migraine.",
        ),
        (
            "When do I video call my sister?",
            "User video calls with her sister Carolina every friday evening to catch up.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let mut original = fact("answer", text);
        original.pinned = true;
        bank.commit(original.clone()).await.unwrap();
        for i in 0..96 {
            bank.commit(fact(
                &format!("noise-{i}"),
                &format!("The visitor asks {query} and discusses workshop number {i}."),
            ))
            .await
            .unwrap();
        }
        assert_eq!(
            bank.recall(query, None, "", 1, 30, false).await[0].0.id,
            "answer"
        );
        let (mut hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(evidence["answer"].life_attribute.as_ref().unwrap()["matched"] == true);
        hits.sort_by_key(|(m, _)| m.id == "answer");
        assert!(hits.len() > 20);
        assert_eq!(
            select(hits, &evidence, &HashSet::new(), 1, query)[0].0.id,
            "answer"
        );
        let session = format!("life-crowded-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), json!(["answer"]));
        rules_preserved(&response);
        assert_eq!(bank.get("answer").await.unwrap(), original);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn life_original_six_complete_bank_authentic_lanes() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let groups = [
        "q-pt-life-par-romance",
        "q-evt-answer-style",
        "q-pt-life-par-sono",
        "q-pt-life-par-hospedagem",
        "q-pt-life-par-olaria",
        "q-pt-life-par-remedio",
        "q-pt-life-par-artistas",
        "q-pt-life-par-mana",
    ];
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
        let cache: Option<Value> = raw.map(|s| serde_json::from_str(s).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let boundary = if let Some(c) = &cache {
            Some(super::super::cached_boundary::boundary(dir.path(), c.clone()).await)
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
        for f in corpus["memories"].as_array().unwrap() {
            let id = f["key"].as_str().unwrap();
            let text = f["text"].as_str().unwrap();
            let mut m = fact(id, text);
            m.tags = serde_json::from_value(f["tags"].clone()).unwrap();
            m.entities = serde_json::from_value(f["entities"].clone()).unwrap();
            m.pinned = f["pinned"].as_bool().unwrap_or(false);
            m.corroboration = f["corroboration"].as_u64().unwrap_or(0) as u32;
            m.confidence = crate::types::Confidence::Extracted;
            m.source = Some(crate::types::Provenance {
                session_id: Some("synthetic-activity-source".into()),
                entry_id: Some(format!("entry-{id}")),
                agent: None,
            });
            let at = now_ms().saturating_sub(f["age_days"].as_u64().unwrap() * 86_400_000);
            m.created_at = at;
            m.updated_at = at;
            m.stated_at = Some(at);
            bank.commit(m).await.unwrap();
            if let Some(c) = &cache {
                assert_eq!(c["items"][id]["text"], text);
                bank.set_vector(
                    id,
                    c["model"].as_str().unwrap(),
                    text,
                    serde_json::from_value(c["items"][id]["v"].clone()).unwrap(),
                )
                .await
                .unwrap();
            }
        }
        let before = std::fs::read(dir.path().join(bank_name).join("memories.jsonl")).unwrap();
        for (n, id) in groups.iter().enumerate() {
            let q = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == *id)
                .unwrap();
            let query = q["text"].as_str().unwrap();
            let mut expected: Vec<String> = serde_json::from_value(q["expect"].clone()).unwrap();
            expected.sort();
            let vector: Option<Vec<f32>> = cache.as_ref().map(|c| {
                assert_eq!(c["items"][*id]["text"], query);
                serde_json::from_value(c["items"][*id]["v"].clone()).unwrap()
            });
            let model = cache
                .as_ref()
                .and_then(|c| c["model"].as_str())
                .unwrap_or("");
            let public = bank
                .recall(query, vector.as_deref(), model, 6, 30, false)
                .await;
            assert!(
                expected
                    .iter()
                    .all(|id| public.iter().any(|(m, _)| &m.id == id)),
                "public {lane}/{id}"
            );
            let (hits, evidence) = bank
                .recall_evidence(query, vector.as_deref(), model, usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&hits, &evidence);
            for id in &expected {
                assert!(
                    reasons.get(id).is_some_and(|r| r.supported()),
                    "proof {lane}/{id}"
                );
            }
            println!(
                "{}",
                json!({"diagnostic":"life-original","lane":lane,"query_id":id,"expected":expected,"proofs":expected.iter().map(|id|json!({"id":id,"proof":evidence.get(id).and_then(|e|e.life_attribute.as_ref())})).collect::<Vec<_>>()})
            );
            let mut actual = select(hits, &evidence, &HashSet::new(), 6, query)
                .into_iter()
                .map(|(m, _)| m.id)
                .collect::<Vec<_>>();
            actual.sort();
            assert_eq!(actual, expected, "select {lane}/{id}");
            let session = format!("life-original-{lane}-{n}");
            let input:PreGenerateInput=serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"step":0,"metadata":{"memory_bank":bank_name},"recall_context":context(&session,"direct",query),"generate":{"system_prompt":"Offline activity","messages":[{"role":"user","content":query}]}})).unwrap();
            let response = serde_json::to_value(pre_generate(&deps, input).await.unwrap()).unwrap();
            let mut actual: Vec<String> = serde_json::from_value(ids(&response)).unwrap();
            actual.sort();
            assert_eq!(actual, expected, "hook {lane}/{id}");
            rules_preserved(&response);
        }
        assert_eq!(
            std::fs::read(dir.path().join(bank_name).join("memories.jsonl")).unwrap(),
            before
        );
        if let Some(b) = &boundary {
            let calls = b.calls.lock().unwrap();
            assert_eq!(calls.len(), groups.len());
            assert!(!calls.iter().any(|c| c.starts_with("UNEXPECTED:")));
        }
    }
}

/// Independently labelled minimal pairs run before selection and through the
/// public hook; no synthetic embedding or policy self-oracle supplies labels.
#[tokio::test]
async fn life_preference_and_performer_neutral_minimal_pairs() {
    let mut count = 0;
    for (query, positive, negatives) in [
        ("Sabe qual romance colombiano eu mais amo?",
         "O livro favorito da usuária é A Ponte de Vidro, de Lina Ferraz.",
         vec!["O filme favorito da usuária é A Ponte de Vidro, de Lina Ferraz.",
              "O livro favorito da visitante é A Ponte de Vidro, de Lina Ferraz.",
              "A usuária está lendo A Ponte de Vidro, de Lina Ferraz.",
              "O formato favorito da usuária é capa dura, de Lina Ferraz."]),
        ("What is my favorite novel?",
         "User's favorite book is The Amber Orchard by Mira Solano.",
         vec!["The visitor's favorite book is The Amber Orchard by Mira Solano.",
              "User's favorite film is The Amber Orchard by Mira Solano.",
              "The Amber Orchard's favorite book is User by Mira Solano.",
              "User reads The Amber Orchard by Mira Solano."]),
        ("Do I prefer short answers or long explanations from you?",
         "User prefers short answers from the assistant.",
         vec!["The visitor prefers short answers from the assistant.",
              "The assistant prefers short answers from user.",
              "User prefers short speeches from the assistant.",
              "User prefers short code comments from the assistant.",
              "User prefers short answers from the visitor."]),
        ("Do I prefer long explanations or short answers from you?",
         "User prefers long, detailed explanations from you that include examples and explain each step.",
         vec!["User writes long, detailed explanations for the visitor.",
              "User prefers long public speeches.",
              "User prefers long source code.",
              "User never prefers long explanations from you."]),
        ("Which artists do I listen to while coding?",
         "User listens to music while programming, especially singers mira vale and tavi reed.",
         vec!["User listens to music while programming, especially Heavy Metal.",
              "User listens to music while programming, especially heavy metal.",
              "User listens to software while programming, especially Heavy Metal.",
              "User listens to music while programming, especially software Heavy Metal.",
              "User listens to music while programming, especially Zephoria.",
              "User listens to music while programming, especially Jazz and Blues."]),
    ] {
        for (text, expected) in std::iter::once((positive.to_owned(), true))
            .chain(negatives.into_iter().map(|s|(s.to_owned(),false)))
            .chain([
                (format!("If {positive}"),false),
                (format!("User quotes: \"{positive}\""),false),
                (format!("User reports that {positive}"),false),
                (format!("{positive} The visitor agrees."),false),
            ]) {
            let proof = life_evidence(query, &text).unwrap();
            assert_eq!(proof["matched"],expected,"{query}/{text}: {proof}");
            if proof["kind"] == "favorite_literary_work_context" {
                assert_eq!(proof["certifies_work_nationality"],false);
                assert_eq!(proof["certifies_work_is_novel"],false);
            }
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank,_) = store.ensure_bank("hook-relevance",None).await.unwrap();
            bank.set_rule("standing","Preserve the permanent safety rule.").unwrap();
            let original = fact("candidate",&text);
            bank.commit(original.clone()).await.unwrap();
            let (hits,evidence) = bank.recall_evidence(query,None,"",usize::MAX,30,false,0.0).await;
            assert_eq!(admission_reasons(&hits,&evidence).get("candidate").is_some_and(|r|r.supported()),expected,"preselect {query}/{text}");
            let expected_ids = if expected {json!(["candidate"])} else {json!([])};
            assert_eq!(json!(select(hits,&evidence,&HashSet::new(),6,query).iter().map(|(m,_)|m.id.clone()).collect::<Vec<_>>()),expected_ids,"select {text}");
            let session = format!("preference-pair-{count}");
            let response = hook(&deps,&session,json!([{"role":"user","content":query}]),context(&session,"direct",query)).await;
            assert_eq!(ids(&response),expected_ids,"hook {query}/{text}");
            rules_preserved(&response);
            assert_eq!(bank.get("candidate").await.unwrap(),original);
            count += 1;
        }
    }
    assert_eq!(count, 48);
}

#[tokio::test]
async fn life_same_affirmative_music_frame_requires_argument_type() {
    let query = "Which artists do I listen to while coding?";
    for (n, (text, expected)) in [
        (
            "User listens to music while programming, especially Heavy Metal.",
            false,
        ),
        (
            "User listens to music while programming, especially performers Heavy Metal.",
            true,
        ),
        (
            "User listens to music while programming, especially software Heavy Metal.",
            false,
        ),
        (
            "User listens to music while programming, especially performers zephoria.",
            true,
        ),
        (
            "User listens to music while programming, especially Zephoria.",
            false,
        ),
        (
            "User listens to music while programming, especially Black Metal and Death Metal.",
            false,
        ),
        ("User listens to music while programming, especially Silver Estuary.", false),
        ("User listens to music while programming, especially silver estuary.", false),
        ("User listens to music while programming, especially Silver Estuary and Amber Orchard.", false),
        ("User listens to music while programming, especially performers silver estuary and amber orchard.", true),
        ("User listens to music while programming, especially performers Silver Estuary.", true),
        ("User listens to music while programming, especially performers Heavy Metal and Black Metal.", true),
        ("User listens to music while programming, especially performers software Silver Estuary.", false),
        ("User listens to music while programming, especially performers Silver Estuary and genres jazz.", false),
        ("User listens to music while programming, especially genres Silver Estuary.", false),
        ("User listens to software while programming, especially performers Silver Estuary.", false),
        ("User listens to music while programming, especially performers Silver Estuary says User listens.", false),
        ("If User listens to music while programming, especially performers Silver Estuary.", false),
        ("User quotes: User listens to music while programming, especially performers Silver Estuary.", false),
        ("The visitor listens to music while programming, especially performers Silver Estuary.", false),
    ]
    .into_iter()
    .enumerate()
    {
        let proof = life_evidence(query, text).unwrap();
        assert_eq!(proof["matched"], expected, "{text}/{proof}");
        assert_eq!(proof["certifies_performer_type"], expected);
        if expected {
            assert!(proof["performer_type_bytes"].is_array());
            assert_eq!(proof["performer_type_witness"], "source_argument_type_head");
        } else {
            assert!(proof["performer_type_witness"].is_null());
        }
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(fact("candidate", text)).await.unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert_eq!(
            admission_reasons(&hits, &evidence)
                .get("candidate")
                .is_some_and(|r| r.supported()),
            expected,
            "preselect {text}"
        );
        let expected_ids = if expected {
            json!(["candidate"])
        } else {
            json!([])
        };
        assert_eq!(
            json!(select(hits, &evidence, &HashSet::new(), 6, query)
                .iter()
                .map(|(m, _)| m.id.clone())
                .collect::<Vec<_>>()),
            expected_ids
        );
        let session = format!("music-type-pair-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), expected_ids, "hook {text}");
    }
}

#[tokio::test]
async fn life_music_entity_types_keep_assertion_roles_separate() {
    let query = "Which artists do I listen to while coding?";
    // Held-out ordinary entities, ambiguous human/artifact homonyms, opaque
    // unseen names and genre arguments all traverse the same listening frame.
    let cases = [
        (
            "Ella Fitzgerald and Miles Davis",
            true,
            "bounded_musical_entity_taxonomy",
        ),
        (
            "ella fitzgerald and miles davis",
            true,
            "bounded_musical_entity_taxonomy",
        ),
        (
            "Joni Mitchell and Stevie Wonder",
            true,
            "bounded_musical_entity_taxonomy",
        ),
        ("Björk", true, "bounded_musical_entity_taxonomy"),
        ("Heavy Metal", false, "unresolved"),
        ("heavy metal", false, "unresolved"),
        ("Jazz and Blues", false, "unresolved"),
        ("Bossa Nova and Reggae", false, "unresolved"),
        ("Ella Fitzgerald and Jazz", false, "unresolved"),
        ("Miles Davis and Zephoria", false, "unresolved"),
        ("Zephoria", false, "unresolved"),
        ("Silver Estuary and Amber Orchard", false, "unresolved"),
        ("Seal", false, "unresolved"),
        ("Sting", false, "unresolved"),
        ("Genesis", false, "unresolved"),
        ("performers Seal", true, "source_argument_type_head"),
        ("performers Sting", true, "source_argument_type_head"),
        ("performers zephoria", true, "source_argument_type_head"),
        (
            "artists delta lantern and iris meadow",
            true,
            "source_argument_type_head",
        ),
        ("performers Heavy Metal", true, "source_argument_type_head"),
        ("software Ella Fitzgerald", false, "unresolved"),
        ("genres Ella Fitzgerald", false, "unresolved"),
        (
            "performers software Ella Fitzgerald",
            false,
            "source_argument_type_head",
        ),
        (
            "performers Ella Fitzgerald and genres jazz",
            false,
            "source_argument_type_head",
        ),
    ];
    let mut count = 0;
    for (argument, expected, basis) in cases {
        let positive_frame =
            format!("User listens to music while programming, especially {argument}.");
        for (text, admitted) in [
            (positive_frame.clone(), expected),
            (positive_frame.replacen("User", "The visitor", 1), false),
            (positive_frame.replacen("music", "software", 1), false),
            (positive_frame.replacen("programming", "cooking", 1), false),
            (format!("If {positive_frame}"), false),
        ] {
            let proof = life_evidence(query, &text).unwrap();
            assert_eq!(proof["matched"], admitted, "{text}: {proof}");
            assert_eq!(proof["certifies_performer_type"], admitted);
            assert_eq!(proof["certifies_performer_nationality"], false);
            assert_eq!(proof["certifies_singer_subtype"], false);
            if text == positive_frame {
                assert_eq!(proof["music_argument_typing"]["type_basis"], basis);
                assert_eq!(
                    proof["music_argument_typing"]["source_argument_role"],
                    "listening_preference_example"
                );
            }
            if admitted {
                for role in [
                    "owner_bytes",
                    "governor_bytes",
                    "activity_bytes",
                    "value_bytes",
                    "performer_link_bytes",
                ] {
                    assert!(proof[role].is_array(), "{role}: {proof}");
                }
                if basis == "bounded_musical_entity_taxonomy" {
                    assert!(proof["performer_type_bytes"].is_null());
                    assert!(proof["performer_type_witness"].is_null());
                }
            }
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let original = fact("candidate", &text);
            bank.commit(original.clone()).await.unwrap();
            let (hits, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            assert_eq!(
                admission_reasons(&hits, &evidence)
                    .get("candidate")
                    .is_some_and(|r| r.supported()),
                admitted,
                "preselect {text}"
            );
            let expected_ids = if admitted {
                json!(["candidate"])
            } else {
                json!([])
            };
            assert_eq!(
                json!(select(hits, &evidence, &HashSet::new(), 6, query)
                    .iter()
                    .map(|(m, _)| m.id.clone())
                    .collect::<Vec<_>>()),
                expected_ids,
                "select {text}"
            );
            let session = format!("music-entity-{count}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            assert_eq!(ids(&response), expected_ids, "hook {text}");
            rules_preserved(&response);
            assert_eq!(bank.get("candidate").await.unwrap(), original);
            count += 1;
        }
    }
    assert_eq!(count, 120);
}

#[tokio::test]
async fn life_original_mpb_resolves_type_without_inventing_a_source_head() {
    let text =
        "A usuária escuta muita MPB enquanto programa, principalmente Gal Costa e Caetano Veloso.";
    for (n, query) in [
        "Que artistas brasileiros tocam no meu fone quando estou codando?",
        "Which artists do I listen to while coding?",
    ]
    .into_iter()
    .enumerate()
    {
        let proof = life_evidence(query, text).unwrap();
        assert_eq!(proof["matched"], true);
        assert_eq!(
            proof["music_argument_typing"]["type_basis"],
            "bounded_musical_entity_taxonomy"
        );
        assert_eq!(
            proof["music_argument_typing"]["entities"],
            json!([
                {"value":"gal costa","lexical_type":"performer"},
                {"value":"caetano veloso","lexical_type":"performer"}
            ])
        );
        assert!(proof["performer_type_bytes"].is_null());
        assert!(proof["performer_type_witness"].is_null());
        assert_eq!(proof["certifies_performer_type"], true);
        assert_eq!(proof["certifies_performer_nationality"], false);
        assert_eq!(proof["certifies_singer_subtype"], false);
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(fact("original", text)).await.unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(admission_reasons(&hits, &evidence)["original"].supported());
        assert_eq!(
            json!(select(hits, &evidence, &HashSet::new(), 6, query)
                .iter()
                .map(|(m, _)| m.id.clone())
                .collect::<Vec<_>>()),
            json!(["original"])
        );
        let session = format!("music-original-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), json!(["original"]));
    }
}
