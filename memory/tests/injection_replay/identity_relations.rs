//! Identity obligations are audited before select; no inference or new vectors.
use super::*;
use crate::store::query_projection::{identity_equivalent, identity_evidence};

pub(super) fn fact(id: &str, text: &str) -> Memory {
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
async fn identity_neutral_owner_attribute_and_root_controls() {
    let cases = [
        (
            "What's my job title at work?",
            "User is a senior acoustics engineer at Juniper Labs.",
            "User is a payment gateway at Juniper Labs.",
        ),
        (
            "Which employer do I work for?",
            "User works for Juniper Labs.",
            "User's colleague works for Juniper Labs.",
        ),
        (
            "What's my partner's name?",
            "User's partner is Noor, a landscape designer she has been dating since 2021.",
            "User's sister is Noor, a landscape designer she has been dating since 2021.",
        ),
        (
            "What's my sister's name?",
            "User's sister is Imani.",
            "User's partner is Imani.",
        ),
        (
            "Which languages do I speak?",
            "User speaks Catalan and Estonian fluently.",
            "User teaches Catalan and Estonian fluently.",
        ),
        (
            "Where was I born?",
            "User was born in Cuenca, Ecuador.",
            "User works in Cuenca, Ecuador.",
        ),
        (
            "qual é a minha cidade natal?",
            "User was born in Mombasa, Kenya.",
            "User's colleague was born in Mombasa, Kenya.",
        ),
        (
            "quando é o meu aniversário?",
            "A usuária nasceu em 17 de abril de 1991.",
            "A irmã da usuária nasceu em 17 de abril de 1991.",
        ),
    ];
    let mut executed = 0;
    for (query, positive, wrong_attribute) in cases {
        let foreign =
            positive
                .replacen("User", "The visitor", 1)
                .replacen("A usuária", "A visitante", 1);
        let negated = positive
            .replacen(" is ", " is not ", 1)
            .replacen(" works ", " never works ", 1)
            .replacen(" speaks ", " never speaks ", 1)
            .replacen(" was born ", " was not born ", 1)
            .replacen(" nasceu ", " não nasceu ", 1);
        for (text, expected) in [
            (positive.to_owned(), true),
            (wrong_attribute.to_owned(), false),
            (foreign, false),
            (negated, false),
            (format!("User reports that {positive}"), false),
            (format!("User quotes: \"{positive}\""), false),
            (format!("If {positive} ask the visitor."), false),
            ("Noor's partner is user.".into(), false),
            ("The city's name is Noor.".into(), false),
        ] {
            let direct = identity_evidence(query, &text, &[], now_ms()).unwrap();
            assert_eq!(direct["matched"], expected, "{query} / {text}: {direct}");
            if expected {
                let proof = &direct["assertion"];
                let span = proof["value_bytes"].as_array().unwrap();
                let normalized = text.trim().to_lowercase();
                assert_eq!(
                    &normalized
                        [span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize],
                    proof["value"].as_str().unwrap()
                );
                assert!(proof["owner_bytes"].is_array());
                assert!(proof["governor_bytes"].is_array());
            }
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = fact("identity", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let public = bank.recall(query, None, "", 30, 30, false).await;
            if expected {
                assert!(public.iter().any(|(m, _)| m.id == "identity"));
            }
            let (hits, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            if let Some(e) = evidence.get("identity") {
                assert_eq!(e.identity_attribute.as_ref(), Some(&direct));
            }
            let reasons = admission_reasons(&hits, &evidence);
            assert_eq!(
                reasons.get("identity").is_some_and(|r| r.supported()),
                expected,
                "preselect: {query} / {text}"
            );
            println!(
                "{}",
                json!({"diagnostic":"identity-neutral","stage":"before_select","query":query,"text":text,"proof":direct,"reasons":reasons})
            );
            assert_eq!(
                select(hits, &evidence, &HashSet::new(), 6, query).len(),
                usize::from(expected)
            );
            let session = format!("identity-neutral-{executed}");
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
                    json!(["identity"])
                } else {
                    json!([])
                },
                "{query} / {text}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("identity").await.unwrap(), original);
            executed += 1;
        }
    }
    assert_eq!(executed, 72);
}

#[test]
fn identity_request_grammar_does_not_certify_foreign_or_quoted_owners() {
    for query in [
        "What's the visitor's partner's name?",
        "If I had a partner, what's my partner's name?",
        "User quotes: What's my partner's name?",
        "What is the company's name?",
        "Which room is my workshop in?",
        "How many people sit on the astronomy team?",
        "What's my partner's name? What's my sister's name?",
    ] {
        assert!(
            identity_evidence(query, "User's partner is Noor.", &[], now_ms()).is_none(),
            "{query}"
        );
    }
    for text in [
        "The company's name is Noor.",
        "User's company is Noor.",
        "Noor's partner is user.",
        "The user named Noor's sister is Imani.",
    ] {
        assert_eq!(
            identity_evidence("What's my partner's name?", text, &[], now_ms()).unwrap()["matched"],
            false,
            "{text}"
        );
    }
}

#[test]
fn identity_dates_require_calendar_and_matching_current_reference() {
    let now = chrono::NaiveDate::from_ymd_opt(2031, 8, 20)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis() as u64;
    let birth = "User was born on 1998-04-17.";
    let age = "User turned 33 years old in April 2031.";
    for query in ["how old am I now?", "when is my birthday?"] {
        assert_eq!(
            identity_evidence(query, birth, &[], now).unwrap()["matched"],
            true
        );
        let proof = identity_evidence(query, age, &[birth], now).unwrap();
        assert_eq!(proof["matched"], true);
        assert_eq!(proof["temporal_witness"]["current_date"], "2031-08-20");
        assert_eq!(proof["temporal_witness"]["birth_date"], "1998-04-17");
        for (text, witnesses) in [
            (age, vec![]),
            (age, vec!["User was born on 1999-04-17."]),
            (age, vec!["The visitor was born on 1998-04-17."]),
            ("User turned 34 years old in April 2031.", vec![birth]),
            ("User turned 33 years old in March 2031.", vec![birth]),
            ("User turned 32 years old in April 2030.", vec![birth]),
            ("User was born on 1998-02-30.", vec![]),
            ("User was born on 2032-04-17.", vec![]),
        ] {
            assert_eq!(
                identity_evidence(query, text, &witnesses, now).unwrap()["matched"],
                false,
                "{query} / {text}"
            );
        }
    }
}

#[tokio::test]
async fn identity_partner_equivalence_preserves_details_pins_sources_and_bank() {
    let a = "User's partner is Noor, a landscape designer she has been dating since 2021.";
    let b = "Noor, a landscape designer, has been the user's partner since 2021.";
    assert!(identity_equivalent(a, b));
    for text in [
        "User's partner is Noor.",
        "User's partner is Noor, a graphic designer she has been dating since 2021.",
        "User's partner is Noor, a landscape designer she has been dating since 2020.",
        "User's sister is Noor, a landscape designer she has been dating since 2021.",
        "User's partner is Imani, a landscape designer she has been dating since 2021.",
    ] {
        assert!(!identity_equivalent(a, text));
    }
    for mode in ["equivalent", "details", "pin", "source"] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        let mut first = fact("first", a);
        first.corroboration = 2;
        let mut second = fact(
            "second",
            if mode == "details" {
                "User's partner is Noor, a graphic designer she has been dating since 2021."
            } else {
                b
            },
        );
        if mode == "pin" {
            second.pinned = true;
        }
        if mode == "source" {
            second.source = Some(crate::types::Provenance {
                session_id: Some("distinct-source".into()),
                entry_id: Some("entry".into()),
                ..Default::default()
            });
        }
        bank.commit(first.clone()).await.unwrap();
        bank.commit(second.clone()).await.unwrap();
        let query = "What's my partner's name?";
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        let selected = select(hits, &evidence, &HashSet::new(), 6, query);
        assert_eq!(
            selected.len(),
            if mode == "equivalent" { 1 } else { 2 },
            "{mode}: {selected:?}"
        );
        if mode == "equivalent" {
            assert_eq!(selected[0].0.id, "first");
        }
        assert_eq!(bank.get("first").await.unwrap(), first);
        assert_eq!(bank.get("second").await.unwrap(), second);
    }
}

#[tokio::test]
async fn identity_does_not_bypass_requested_count_or_source_authority() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut ungrounded = fact("ungrounded", "User works for Juniper Labs.");
    ungrounded.confidence = crate::types::Confidence::Extracted;
    bank.commit(ungrounded).await.unwrap();
    bank.commit(fact(
        "team",
        "User is a senior acoustics engineer on the astronomy team at Juniper Labs.",
    ))
    .await
    .unwrap();
    for (n, query) in [
        "Which employer do I work for?",
        "How many people sit on the astronomy team?",
    ]
    .into_iter()
    .enumerate()
    {
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        let reasons = admission_reasons(&hits, &evidence);
        assert!(!reasons.get("ungrounded").is_some_and(|r| r.supported()));
        if n == 1 {
            assert!(reasons.values().all(|r| !r.supported()));
        }
        let response = hook(
            &deps,
            &format!("identity-boundary-{n}"),
            json!([{"role":"user","content":query}]),
            context(&format!("identity-boundary-{n}"), "direct", query),
        )
        .await;
        if n == 1 {
            assert_eq!(ids(&response), json!([]));
        }
        assert!(!ids(&response)
            .as_array()
            .unwrap()
            .contains(&json!("ungrounded")));
        rules_preserved(&response);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_original_seven_complete_bank_lexical_and_authentic_caches() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let groups = [
        ("q-id-job-title", vec!["id-role", "id-empresa"]),
        ("q-id-partner", vec!["id-partner"]),
        ("q-id-how-old", vec!["id-age", "id-nascimento"]),
        ("q-id-languages-en", vec!["id-idiomas"]),
        ("q-id-aniversario", vec!["id-nascimento", "id-age"]),
        ("q-id-empresa", vec!["id-empresa", "id-role"]),
        ("q-id-cidade-natal", vec!["id-birthplace"]),
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
            m.corroboration = f["corroboration"].as_u64().unwrap_or(0) as u32;
            m.pinned = f["pinned"].as_bool().unwrap_or(false);
            m.confidence = crate::types::Confidence::Extracted;
            m.source = Some(crate::types::Provenance {
                session_id: Some("synthetic-identity-source".into()),
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
        for (n, (query_id, expected)) in groups.iter().enumerate() {
            let q = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == *query_id)
                .unwrap();
            let query = q["text"].as_str().unwrap();
            let vector: Option<Vec<f32>> = cache.as_ref().map(|c| {
                assert_eq!(c["items"][*query_id]["text"], query);
                serde_json::from_value(c["items"][*query_id]["v"].clone()).unwrap()
            });
            let model = cache
                .as_ref()
                .and_then(|c| c["model"].as_str())
                .unwrap_or("");
            let (hits, evidence) = bank
                .recall_evidence(query, vector.as_deref(), model, usize::MAX, 30, false, 0.0)
                .await;
            let public = bank
                .recall(query, vector.as_deref(), model, 6, 30, false)
                .await;
            for id in expected {
                assert!(
                    public.iter().any(|(m, _)| m.id == *id),
                    "public {lane}/{query_id}/{id}"
                );
            }
            let reasons = admission_reasons(&hits, &evidence);
            for id in expected {
                if !reasons.get(*id).is_some_and(|r| r.supported()) {
                    failures.push(format!("{lane}/{query_id}/{id}: missing preselect proof"));
                }
            }
            println!(
                "{}",
                json!({"diagnostic":"identity-original","stage":"before_select","lane":lane,"query_id":query_id,"expected":expected,"proofs":expected.iter().map(|id|json!({"id":id,"proof":evidence.get(*id).and_then(|e|e.identity_attribute.as_ref())})).collect::<Vec<_>>(),"reasons":reasons})
            );
            let selected = select(hits, &evidence, &HashSet::new(), 6, query);
            let mut actual = selected
                .iter()
                .map(|(m, _)| m.id.as_str())
                .collect::<Vec<_>>();
            actual.sort();
            let mut want = expected.clone();
            want.sort();
            if actual != want {
                failures.push(format!("{lane}/{query_id}: {actual:?} != {want:?}"));
            }
            let session = format!("identity-original-{lane}-{n}");
            let input:PreGenerateInput=serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"step":0,"metadata":{"memory_bank":bank_name},"recall_context":context(&session,"direct",query),"generate":{"system_prompt":"Offline identity","messages":[{"role":"user","content":query}]}})).unwrap();
            let response = serde_json::to_value(pre_generate(&deps, input).await.unwrap()).unwrap();
            let mut hook_ids: Vec<String> = serde_json::from_value(ids(&response)).unwrap();
            hook_ids.sort();
            if hook_ids != want {
                failures.push(format!("hook {lane}/{query_id}: {hook_ids:?} != {want:?}"));
            }
            rules_preserved(&response);
        }
        assert_eq!(
            std::fs::read(dir.path().join(bank_name).join("memories.jsonl")).unwrap(),
            before
        );
        if let Some(b) = &boundary {
            let calls = b.calls.lock().unwrap();
            assert_eq!(calls.len(), 7, "{lane}: {calls:?}");
            assert!(!calls.iter().any(|c| c.starts_with("UNEXPECTED:")));
        }
    }
    assert!(
        failures.is_empty(),
        "all identity original lanes: {failures:?}"
    );
}

#[tokio::test]
async fn identity_nested_possessors_only_prove_the_requested_person_and_name() {
    for (n, role) in [
        "partner", "spouse", "sister", "brother", "daughter", "father",
    ]
    .iter()
    .enumerate()
    {
        let query = format!("What's my {role}'s name?");
        for (j, (text, expected)) in [
            (format!("User's {role}'s name is Amari."), true),
            (format!("My {role}'s name is Laleh."), true),
            (format!("The user's {role}'s name is Sora."), true),
            (format!("The visitor's {role}'s name is Amari."), false),
            (format!("User's {role}'s colleague's name is Amari."), false),
            ("User's name is Amari.".into(), false),
            (format!("Amari's {role}'s name is User."), false),
            (format!("User's {role}'s name is not Amari."), false),
            (format!("User's {role}'s name is unknown."), false),
            (format!("User's {role}'s name is possibly Amari."), false),
            (format!("User's {role}'s name is Amari?"), false),
            (format!("If user's {role}'s name is Amari."), false),
            (
                format!("User reports that user's {role}'s name is Amari."),
                false,
            ),
            (format!("\"User's {role}'s name is Amari.\""), false),
        ]
        .into_iter()
        .enumerate()
        {
            let proof = identity_evidence(&query, &text, &[], now_ms()).unwrap();
            assert_eq!(proof["matched"], expected, "{query}: {text}");
            if expected {
                let assertion = &proof["assertion"];
                let normalized = text.to_lowercase();
                let span = assertion["owner_bytes"].as_array().unwrap();
                let owner = &normalized
                    [span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize];
                assert!(matches!(owner, "user's" | "the user's" | "my"));
                let span = assertion["value_bytes"].as_array().unwrap();
                assert_eq!(
                    &normalized
                        [span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize],
                    assertion["value"].as_str().unwrap()
                );
            }
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            bank.commit(fact("nested", &text)).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (hits, evidence) = bank
                .recall_evidence(&query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            assert_eq!(
                admission_reasons(&hits, &evidence)
                    .get("nested")
                    .is_some_and(|r| r.supported()),
                expected
            );
            assert_eq!(
                select(hits, &evidence, &HashSet::new(), 6, &query).len(),
                usize::from(expected)
            );
            let session = format!("nested-{n}-{j}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", &query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["nested"])
                } else {
                    json!([])
                },
                "{text}"
            );
            rules_preserved(&response);
        }
    }
}

#[tokio::test]
async fn identity_typed_discovery_precedes_bounded_noise_without_score_or_vector_proof() {
    for (n, (query, text)) in [
        (
            "What's my partner's name?",
            "User's partner's name is Amari.",
        ),
        (
            "Which employer do I work for?",
            "User works as a cartographer at Willow Cooperative.",
        ),
        ("Where was I born?", "User was born in Valdivia, Chile."),
        (
            "quando é o meu aniversário?",
            "A usuária nasceu em 17 de abril de 1991.",
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
        bank.commit(fact("answer", text)).await.unwrap();
        for i in 0..96 {
            bank.commit(fact(
                &format!("noise-{i}"),
                &format!("The visitor asks {query} and discusses a workshop number {i}."),
            ))
            .await
            .unwrap();
        }
        let public = bank.recall(query, None, "", 1, 30, false).await;
        assert_eq!(public[0].0.id, "answer", "finite public recall {query}");
        let (mut hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(evidence["answer"].identity_attribute.as_ref().unwrap()["matched"] == true);
        // Deliberately adversarial input order/scores, not manufactured vectors.
        hits.sort_by_key(|(m, _)| m.id == "answer");
        assert!(hits.len() > 20);
        assert_eq!(hits.last().unwrap().0.id, "answer");
        assert_eq!(
            select(hits, &evidence, &HashSet::new(), 1, query)[0].0.id,
            "answer"
        );
        let session = format!("crowded-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), json!(["answer"]));
        rules_preserved(&response);
    }
}

#[tokio::test]
async fn identity_complete_equivalence_keeps_foreign_sources_details_and_conflicting_names() {
    let query = "What's my partner's name?";
    let first_text = "User's partner is Amari, a cartographer she has been dating since 2020.";
    let twin_text = "Amari, a cartographer, has been the user's partner since 2020.";
    for mode in [
        "same-session",
        "foreign-session",
        "foreign-agent",
        "extra-detail",
        "conflicting-name",
        "pinned",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let mut first = fact("first", first_text);
        first.corroboration = 3;
        let mut second = fact(
            "second",
            match mode {
                "extra-detail" => {
                    "User's partner is Amari, a marine cartographer she has been dating since 2020."
                }
                "conflicting-name" => {
                    "User's partner is Sora, a cartographer she has been dating since 2020."
                }
                _ => twin_text,
            },
        );
        first.source = Some(crate::types::Provenance {
            session_id: Some("conversation".into()),
            entry_id: Some("entry-a".into()),
            agent: None,
        });
        second.source = Some(crate::types::Provenance {
            session_id: Some(
                if mode == "foreign-session" {
                    "other"
                } else {
                    "conversation"
                }
                .into(),
            ),
            entry_id: Some("entry-b".into()),
            agent: if mode == "foreign-agent" {
                Some("other-agent".into())
            } else {
                None
            },
        });
        first.confidence = crate::types::Confidence::Extracted;
        second.confidence = crate::types::Confidence::Extracted;
        second.pinned = mode == "pinned";
        bank.commit(first.clone()).await.unwrap();
        bank.commit(second.clone()).await.unwrap();
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(admission_reasons(&hits, &evidence)
            .values()
            .all(|r| r.supported()));
        let selected = select(hits, &evidence, &HashSet::new(), 6, query);
        assert_eq!(
            selected.len(),
            if mode == "same-session" { 1 } else { 2 },
            "{mode}: {selected:?}"
        );
        if mode == "same-session" {
            assert_eq!(selected[0].0, first);
        }
        assert_eq!(bank.get("first").await.unwrap(), first);
        assert_eq!(bank.get("second").await.unwrap(), second);
        let response = hook(
            &deps,
            mode,
            json!([{"role":"user","content":query}]),
            context(mode, "direct", query),
        )
        .await;
        assert_eq!(
            ids(&response).as_array().unwrap().len(),
            if mode == "same-session" { 1 } else { 2 },
            "{mode}"
        );
        rules_preserved(&response);
    }
}

#[test]
fn identity_unwitnessed_age_and_unknown_occupation_never_become_discovery_proof() {
    let now = chrono::NaiveDate::from_ymd_opt(2031, 8, 20)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis() as u64;
    for text in [
        "User turned 33 years old in April 2031.",
        "User was born on 1998-02-30.",
    ] {
        assert_eq!(
            identity_evidence("when is my birthday?", text, &[], now).unwrap()["matched"],
            false
        );
    }
    assert_eq!(
        identity_evidence(
            "Which employer do I work for?",
            "User works for unknown.",
            &[],
            now
        )
        .unwrap()["matched"],
        false
    );
    assert!(identity_equivalent(
        "User's partner's name is Amari.",
        "My partner is Amari."
    ));
    assert!(!identity_equivalent(
        "User's partner's name is Amari.",
        "The visitor's partner's name is Amari."
    ));
    assert!(!identity_equivalent(
        "User's partner's name is Amari.",
        "User's partner's name is Sora."
    ));
}
