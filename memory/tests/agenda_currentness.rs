//! Offline proof targets, intentionally not executed by the implementation phase.
//! Synthetic anchor is Monday 12 Oct 2026 UTC, NOT the actual current date.
use chrono::{Datelike, Duration, NaiveDate};
use memory::{
    config::WorkerConfig,
    deps::Deps,
    events::Emitter,
    hooks::{self, agenda, PreGenerateInput},
    store::{Bank, Store},
    types::{fingerprint, Confidence, EvaluationClock, Memory, Provenance},
};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};

const PT: &str = "tenho algum compromisso marcado na próxima semana?";
const EN: &str = "what appointments do I have next week?";
fn clock(date: &str) -> EvaluationClock {
    EvaluationClock::synthetic(
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis() as u64,
    )
}
fn fact(text: &str) -> Memory {
    let at = agenda::fixture_clock().now_ms() - 86_400_000;
    Memory {
        id: fingerprint(text),
        text: text.into(),
        entities: vec![],
        tags: vec![],
        confidence: Confidence::Extracted,
        corroboration: 0,
        pinned: false,
        source: Some(Provenance {
            session_id: Some("synthetic-agenda-producer".into()),
            entry_id: Some(fingerprint(text)),
            agent: None,
        }),
        created_at: at,
        updated_at: at,
        stated_at: Some(at),
        invalid_at: None,
        superseded_by: None,
        revision: 0,
    }
}
async fn setup() -> (tempfile::TempDir, Arc<Deps>, Arc<Bank>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().to_path_buf()).unwrap();
    let (bank, _) = store.ensure_bank("agenda", None).await.unwrap();
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
    let cfg = WorkerConfig {
        embeddings_enabled: false,
        extraction_enabled: false,
        recall_limit: 20,
        recall_budget_tokens: 4000,
        ..WorkerConfig::default()
    };
    let deps = Arc::new(Deps {
        hook_binding: Arc::default(),
        emitter: Arc::new(Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(store))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
    });
    (dir, deps, bank)
}
fn input(session: &str, query: &str) -> PreGenerateInput {
    serde_json::from_value(
        json!({"session_id":session,"turn_id":format!("{session}-turn"),
        "metadata":{"memory_bank":"agenda", "now_ms":agenda::fixture_clock().now_ms(),
            "evaluation_clock":"2026-10-12"},
        "now_ms":agenda::fixture_clock().now_ms(),
        "recall_context":{"version":1,"session_id":session,"turn_id":format!("{session}-turn"),
            "source":{"kind":"direct","entry_id":"current"},
            "query":{"entry_id":"current","text":query}},
        "generate":{"system_prompt":"Synthetic offline agenda","messages":[]}}),
    )
    .unwrap()
}
fn delivered(response: hooks::HookResponse) -> (HashSet<String>, String) {
    let annotations = response.annotations.unwrap_or_default();
    let ids: HashSet<String> = serde_json::from_value(
        annotations
            .get("memory_ids")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .unwrap();
    let mutations = response.mutations.unwrap_or_default();
    (ids, serde_json::to_string(&mutations).unwrap())
}
async fn selected(bank: &Bank, query: &str, at: EvaluationClock) -> Vec<Memory> {
    let (hits, evidence) = bank
        .recall_evidence_at(query, None, "", usize::MAX, 30, false, 0.0, at)
        .await;
    hooks::select(hits, &evidence, &HashSet::new(), 20, query)
        .into_iter()
        .map(|(m, _)| m)
        .collect()
}
fn golden(key: &str) -> Memory {
    let corpus: Value = serde_json::from_str(include_str!("recall_quality/golden.json")).unwrap();
    let row = corpus["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["key"] == key)
        .unwrap();
    let mut m = fact(row["text"].as_str().unwrap());
    m.entities = serde_json::from_value(row["entities"].clone()).unwrap();
    m.tags = serde_json::from_value(row["tags"].clone()).unwrap();
    m.pinned = row["pinned"].as_bool().unwrap();
    m.corroboration = row["corroboration"].as_u64().unwrap() as u32;
    m
}

#[tokio::test]
async fn original_october_synthetic_store_hook_two_events_one_dentist() {
    let (_dir, deps, bank) = setup().await;
    bank.set_rule("answer", "Keep the event title and language intact.")
        .unwrap();
    let keys = [
        "evt-dentist-oct15",
        "evt-dentist-moved",
        "dup-dentist-moved",
        "evt-palestra-meetup",
        "evt-passport",
        "evt-prazo-artigo",
        "evt-dermatologista",
    ];
    let facts: Vec<_> = keys.iter().map(|k| golden(k)).collect();
    for m in &facts {
        bank.commit(m.clone()).await.unwrap();
    }
    for (i, query) in [PT, EN].iter().enumerate() {
        let (ids, body) = delivered(
            hooks::pre_generate_at(
                &deps,
                input(&format!("october-{i}"), query),
                agenda::fixture_clock(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(ids.len(), 2, "{body}");
        assert!(ids.contains(&facts[3].id), "talk missing: {body}");
        assert_eq!(
            [&facts[1], &facts[2]]
                .iter()
                .filter(|m| ids.contains(&m.id))
                .count(),
            1
        );
        assert!(!ids.contains(&facts[0].id));
        assert!(body.contains("Filas idempotentes na prática") && body.contains("em inglês"));
        assert!(body.contains("Keep the event title and language intact."));
        for m in &facts {
            assert_eq!(bank.get(&m.id).await.as_ref(), Some(m));
        }
        let (_, evidence) = bank
            .recall_evidence_at(
                query,
                None,
                "",
                usize::MAX,
                30,
                false,
                0.0,
                agenda::fixture_clock(),
            )
            .await;
        let proof = evidence[&facts[3].id].temporal_constraint.as_ref().unwrap();
        assert_eq!(proof["week_start"], "2026-10-19");
        assert_eq!(proof["week_end_exclusive"], "2026-10-26");
        assert_eq!(proof["clock_date"], "2026-10-12");
    }
    for (i, at) in [clock("2027-11-02"), clock("2026-10-05")]
        .into_iter()
        .enumerate()
    {
        let (ids, _) = delivered(
            hooks::pre_generate_at(&deps, input(&format!("outside-{i}"), PT), at)
                .await
                .unwrap(),
        );
        // Neither past nor outside-week anchors revive a rescheduled booking.
        assert!(
            !ids.contains(&facts[1].id)
                && !ids.contains(&facts[2].id)
                && !ids.contains(&facts[3].id)
        );
        assert!(ids.is_empty(), "outside interval must be empty: {ids:?}");
    }
}

/// Full original bank and BOTH existing vector caches: no replacement vectors,
/// widened pool, query-ID admission, live provider or baseline updates.
#[tokio::test]
async fn full_original_bank_cached_vectors_keep_two_dated_events_before_pool_cut() {
    let corpus: Value = serde_json::from_str(include_str!("recall_quality/golden.json")).unwrap();
    for raw in [
        include_str!("recall_quality/vectors-minilm.json"),
        include_str!("recall_quality/vectors.json"),
    ] {
        let cache: Value = serde_json::from_str(raw).unwrap();
        let (_dir, deps, bank) = setup().await;
        let model = cache["model"].as_str().unwrap();
        let at = agenda::fixture_clock();
        for row in corpus["memories"].as_array().unwrap() {
            let key = row["key"].as_str().unwrap();
            let mut m = golden(key);
            let stated = at.now_ms() - row["age_days"].as_u64().unwrap() * 86_400_000;
            m.created_at = stated;
            m.updated_at = stated;
            m.stated_at = Some(stated);
            bank.commit(m.clone()).await.unwrap();
            let v: Vec<f32> = serde_json::from_value(cache["items"][key]["v"].clone()).unwrap();
            assert_eq!(cache["items"][key]["text"], m.text);
            bank.set_vector(&m.id, model, &m.text, v).await.unwrap();
        }
        // Identify the cached query by its exact unchanged text, never by an
        // identifier in product code or by a synthetic replacement vector.
        let query = corpus["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["text"] == PT)
            .unwrap();
        let qv: Vec<f32> =
            serde_json::from_value(cache["items"][query["id"].as_str().unwrap()]["v"].clone())
                .unwrap();
        let (hits, evidence) = bank
            .recall_evidence_at(PT, Some(&qv), model, usize::MAX, 30, false, 0.0, at)
            .await;
        let selected = hooks::select(hits, &evidence, &HashSet::new(), 5, PT);
        let check = |ids: HashSet<String>| {
            assert_eq!(ids.len(), 2, "{model}: {ids:?}");
            assert!(ids.contains(&golden("evt-palestra-meetup").id));
            assert_eq!(
                [golden("evt-dentist-moved"), golden("dup-dentist-moved")]
                    .iter()
                    .filter(|m| ids.contains(&m.id))
                    .count(),
                1
            );
            assert!(!ids.contains(&golden("evt-dentist-oct15").id));
        };
        check(selected.into_iter().map(|(m, _)| m.id).collect());
        // Ordinary Store recall uses include_constraints=false, not injection.
        check(
            bank.recall_at(PT, Some(&qv), model, 5, 30, false, at)
                .await
                .into_iter()
                .map(|(m, _)| m.id)
                .collect(),
        );
        // Actual productive hook, lexical fallback with the same complete bank.
        check(
            delivered(
                hooks::pre_generate_at(&deps, input(&format!("full-{model}"), PT), at)
                    .await
                    .unwrap(),
            )
            .0,
        );
    }
}

#[tokio::test]
async fn public_recall_clock_is_internal_and_hybrid_agenda_is_current() {
    use memory::functions::recall::{self, RecallRequest};
    let (_dir, deps, bank) = setup().await;
    let rows = [
        golden("evt-dentist-moved"),
        golden("dup-dentist-moved"),
        golden("evt-palestra-meetup"),
        golden("evt-dentist-oct15"),
    ];
    for m in &rows {
        bank.commit(m.clone()).await.unwrap();
    }
    let request = |mode: &str| -> RecallRequest {
        serde_json::from_value(json!({"bank":"agenda", "query":PT, "limit":5,
            "mode":mode, "now_ms":agenda::fixture_clock().now_ms(),
            "evaluation_clock":{"now_ms":agenda::fixture_clock().now_ms()}}))
        .unwrap()
    };
    let got = recall::recall_at(&deps, request("hybrid"), agenda::fixture_clock())
        .await
        .unwrap();
    assert_eq!(got.retrieval, "bm25-entity");
    let ids: HashSet<_> = got.memories.iter().map(|m| m.memory.id.clone()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&rows[2].id));
    assert_eq!(
        [&rows[0], &rows[1]]
            .iter()
            .filter(|m| ids.contains(&m.id))
            .count(),
        1
    );
    assert!(!ids.contains(&rows[3].id));
    for outside in [clock("2026-10-05"), clock("2026-11-02")] {
        assert!(recall::recall_at(&deps, request("hybrid"), outside)
            .await
            .unwrap()
            .memories
            .is_empty());
    }
    let today = chrono::Utc::now().date_naive();
    let monday = today + Duration::days(7 - i64::from(today.weekday().num_days_from_monday()));
    let live = fact(&format!(
        "User has an optometrist appointment on {}.",
        monday
    ));
    bank.commit(live.clone()).await.unwrap();
    let real = recall::recall(&deps, request("hybrid")).await.unwrap();
    assert!(real.memories.iter().any(|m| m.memory.id == live.id));
    for old in &rows {
        if agenda::evidence(PT, old, EvaluationClock::wall()).is_none() {
            assert!(
                !real.memories.iter().any(|m| m.memory.id == old.id),
                "real clock must not be supplied by the request"
            );
        }
    }
    // Raw modes preserve their existing contract; semantic without a provider
    // is explicitly unavailable, never secretly substituted by agenda results.
    let semantic = recall::recall_at(&deps, request("semantic"), agenda::fixture_clock())
        .await
        .unwrap();
    assert_eq!(semantic.retrieval, "semantic-unavailable");
    assert!(semantic.memories.is_empty());
    let raw = recall::recall_at(&deps, request("lexical"), agenda::fixture_clock())
        .await
        .unwrap();
    let direct = bank.lexical(PT, 5, false).await;
    assert_eq!(
        raw.memories
            .into_iter()
            .map(|m| m.memory.id)
            .collect::<Vec<_>>(),
        direct.into_iter().map(|(m, _)| m.id).collect::<Vec<_>>()
    );
    for m in &rows {
        assert_eq!(bank.get(&m.id).await.as_ref(), Some(m));
    }
}

#[tokio::test]
async fn neutral_calendar_boundaries_owner_and_commitment_controls() {
    let (_dir, deps, bank) = setup().await;
    let rows = [
        (
            "User has an optometrist appointment at 11:00 on 2030-12-30.",
            true,
        ),
        (
            "A usuária tem consulta em 2 de janeiro de 2031 às 10:00.",
            true,
        ),
        ("User will attend a workshop on 3 January 2031.", true),
        (
            "User has an optometrist appointment at 11:00 on 2030-12-29.",
            false,
        ),
        (
            "User has an optometrist appointment at 11:00 on 2031-01-06.",
            false,
        ),
        (
            "User's mother has an optometrist appointment on 2031-01-02.",
            false,
        ),
        (
            "User said 'I have an optometrist appointment on 2031-01-02'.",
            false,
        ),
        (
            "If the visitor arrives, user has an appointment on 2031-01-02.",
            false,
        ),
        ("User has no appointment on 2031-01-02.", false),
        (
            "User has an appointment on 2031-01-02 if the visitor arrives.",
            false,
        ),
        (
            "User has an appointment on 2031-01-02, not a commitment.",
            false,
        ),
        ("A usuária talvez tenha consulta em 02/01/2031.", false),
        ("A usuária não tem consulta em 02/01/2031.", false),
        ("User was born on 2031-01-02.", false),
        ("User prefers the date format 2031-01-02.", false),
        ("User has an appointment on 2031-02-30.", false),
    ];
    let mut expected = HashSet::new();
    for (text, yes) in rows {
        let m = fact(text);
        if yes {
            expected.insert(m.id.clone());
        }
        bank.commit(m).await.unwrap();
    }
    for query in [PT, EN] {
        let got: HashSet<_> = selected(&bank, query, clock("2030-12-23"))
            .await
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(got, expected);
        let (ids, _) = delivered(
            hooks::pre_generate_at(
                &deps,
                input(&format!("neutral-{}", fingerprint(query)), query),
                clock("2030-12-23"),
            )
            .await
            .unwrap(),
        );
        assert_eq!(ids, expected);
        let req = serde_json::from_value(json!({"bank":"agenda", "query":query})).unwrap();
        let response = memory::functions::recall::recall_at(&deps, req, clock("2030-12-23"))
            .await
            .unwrap();
        assert_eq!(
            response
                .memories
                .into_iter()
                .map(|m| m.memory.id)
                .collect::<HashSet<_>>(),
            expected
        );
    }
    // [23,30) December DOES include the 29th control from the first anchor.
    // Keep that positive assertion, then test an actually disjoint week.
    assert_eq!(
        selected(&bank, EN, clock("2030-12-16"))
            .await
            .into_iter()
            .map(|m| m.id)
            .collect::<HashSet<_>>(),
        HashSet::from([fingerprint(
            "User has an optometrist appointment at 11:00 on 2030-12-29."
        )])
    );
    assert!(selected(&bank, EN, clock("2030-12-09")).await.is_empty());
    assert!(selected(&bank, EN, clock("2031-01-06")).await.is_empty());
}

#[tokio::test]
async fn timezone_offsets_and_dst_use_utc_instants_not_fixed_local_offsets() {
    let (_dir, deps, bank) = setup().await;
    let rows = [
        (
            "User has an appointment on 2026-10-18T23:30:00-03:00.",
            true,
        ),
        (
            "User has an appointment on 2026-10-19T00:30:00+02:00.",
            false,
        ),
        (
            "User has an appointment on 2026-10-25T23:30:00-03:00.",
            false,
        ),
        (
            "User has an appointment on 2026-10-25T01:30:00+02:00.",
            true,
        ),
        (
            "User has an appointment on 2026-10-25T01:30:00+01:00.",
            true,
        ),
        (
            "User has an appointment on 2026-10-25 local time Europe/London.",
            false,
        ),
    ];
    let mut expected = HashSet::new();
    for (text, yes) in rows {
        let m = fact(text);
        if yes {
            expected.insert(m.id.clone());
        }
        bank.commit(m).await.unwrap();
    }
    assert_eq!(expected.len(), 3, "positive controls must actually execute");
    let crossing = fact(rows[0].0);
    let proof = agenda::evidence(EN, &crossing, agenda::fixture_clock()).unwrap();
    assert_eq!(proof["event_date"], "2026-10-19");
    assert_eq!(proof["event_time_utc"], "02:30:00");
    for (text, utc) in [(rows[3].0, "23:30:00"), (rows[4].0, "00:30:00")] {
        assert_eq!(
            agenda::evidence(EN, &fact(text), agenda::fixture_clock()).unwrap()["event_time_utc"],
            utc
        );
    }
    assert_eq!(
        selected(&bank, EN, agenda::fixture_clock())
            .await
            .into_iter()
            .map(|m| m.id)
            .collect::<HashSet<_>>(),
        expected
    );
    let (ids, _) = delivered(
        hooks::pre_generate_at(
            &deps,
            input("utc-positive-hook", EN),
            agenda::fixture_clock(),
        )
        .await
        .unwrap(),
    );
    assert_eq!(ids, expected);
    let req = serde_json::from_value(json!({"bank":"agenda", "query":EN})).unwrap();
    let response = memory::functions::recall::recall_at(&deps, req, agenda::fixture_clock())
        .await
        .unwrap();
    assert_eq!(
        response
            .memories
            .into_iter()
            .map(|m| m.memory.id)
            .collect::<HashSet<_>>(),
        expected
    );
}

#[tokio::test]
async fn distinct_date_time_title_detail_source_and_pinned_are_not_twins() {
    let (_dir, _deps, bank) = setup().await;
    let a = golden("evt-dentist-moved");
    let b = golden("dup-dentist-moved");
    assert_eq!(agenda::equivalent(&a, &b), Some(true));
    let talk = golden("evt-palestra-meetup");
    let mut rows = vec![
        a.clone(),
        b,
        talk.clone(),
        fact("User has a dentist appointment at 9:30 on 2026-10-21."),
        fact("User has a dentist appointment at 10:30 on 2026-10-20."),
        fact("User has a dentist appointment at 9:30 on 2026-10-20, with a new clinic."),
        fact(
            &talk
                .text
                .replace("Filas idempotentes na prática", "Tracing em produção"),
        ),
    ];
    let mut other_source = a.clone();
    other_source.id = "independent-source".into();
    other_source.source.as_mut().unwrap().session_id = Some("another-producer".into());
    assert_eq!(agenda::equivalent(&a, &other_source), Some(false));
    rows.push(other_source);
    let mut pinned = a.clone();
    pinned.id = "pinned-source".into();
    pinned.pinned = true;
    assert_eq!(agenda::equivalent(&a, &pinned), Some(false));
    rows.push(pinned);
    for m in &rows {
        bank.commit(m.clone()).await.unwrap();
    }
    let got = selected(&bank, EN, agenda::fixture_clock()).await;
    assert_eq!(got.len(), rows.len() - 1);
    assert!(got.iter().any(|m| m.pinned));
    for m in &rows {
        assert_eq!(bank.get(&m.id).await.as_ref(), Some(m));
    }
}

#[tokio::test]
async fn real_wall_wrapper_ignores_wire_clock_and_has_nonzero_live_control() {
    let (_dir, deps, bank) = setup().await;
    let today = chrono::Utc::now().date_naive();
    let monday = today + Duration::days(7 - i64::from(today.weekday().num_days_from_monday()));
    let live = fact(&format!(
        "User has an optometrist appointment at 11:00 on {}.",
        monday + Duration::days(2)
    ));
    let old = fact(&format!(
        "User has an optometrist appointment at 11:00 on {}.",
        today - Duration::days(14)
    ));
    bank.commit(live.clone()).await.unwrap();
    let future = fact(&format!(
        "User has an optometrist appointment at 11:00 on {}.",
        monday + Duration::days(14)
    ));
    bank.commit(old.clone()).await.unwrap();
    bank.commit(future.clone()).await.unwrap();
    let (ids, _) = delivered(
        hooks::pre_generate(&deps, input("real-now", EN))
            .await
            .unwrap(),
    );
    assert_eq!(ids, HashSet::from([live.id]));
    assert!(!ids.contains(&old.id));
    assert!(!ids.contains(&future.id));
    // Fixed October controls are valid only when outside the ACTUAL next week;
    // this remains robust even if the checker is run in October 2026 someday.
    for key in ["evt-dentist-moved", "evt-palestra-meetup"] {
        let m = golden(key);
        let proof = agenda::evidence(EN, &m, EvaluationClock::wall());
        bank.commit(m.clone()).await.unwrap();
        let (ids, _) = delivered(
            hooks::pre_generate(&deps, input(&format!("real-{key}"), EN))
                .await
                .unwrap(),
        );
        assert_eq!(ids.contains(&m.id), proof.is_some());
    }
}

#[tokio::test]
async fn age32_year2026_clock_effects_preserve_original_facts() {
    let (_dir, _deps, bank) = setup().await;
    let age = golden("id-age");
    let birth = golden("id-nascimento");
    assert_eq!(age.text, "User turned 32 years old in March 2026.");
    assert_eq!(birth.text, "A usuária nasceu em 22 de março de 1994.");
    bank.commit(age.clone()).await.unwrap();
    bank.commit(birth.clone()).await.unwrap();
    for (date, yes) in [
        ("2026-10-12", true),
        ("2026-02-01", false),
        ("2027-10-12", false),
    ] {
        let (_, ev) = bank
            .recall_evidence_at(
                "how old am I?",
                None,
                "",
                usize::MAX,
                30,
                false,
                0.0,
                clock(date),
            )
            .await;
        let proof = ev[&age.id].identity_attribute.as_ref().unwrap();
        assert_eq!(proof["matched"], yes, "{date}: {proof}");
        assert_eq!(proof["current_date"], date);
        if yes {
            assert_eq!(proof["temporal_witness"]["birth_date"], "1994-03-22");
        }
    }
    assert_eq!(bank.get(&age.id).await.as_ref(), Some(&age));
    assert_eq!(bank.get(&birth.id).await.as_ref(), Some(&birth));
}

#[tokio::test]
async fn rescheduled_destination_outside_week_does_not_revive_old_booking() {
    let (_dir, _deps, bank) = setup().await;
    let old = fact("User has a dentist appointment at 9:30 on 2032-05-04.");
    let mut moved =
        fact("User moved her dentist appointment from 2032-05-04 to 9:30 on 2032-05-18.");
    moved.stated_at = Some(old.last_stated() + 1);
    assert!(agenda::superseded(&old, &moved));
    bank.commit(old.clone()).await.unwrap();
    bank.commit(moved.clone()).await.unwrap();
    assert!(selected(&bank, EN, clock("2032-04-26")).await.is_empty());
    let got = selected(&bank, EN, clock("2032-05-10")).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, moved.id);
    let mut foreign = moved.clone();
    foreign.source.as_mut().unwrap().session_id = Some("foreign".into());
    assert!(!agenda::superseded(&old, &foreign));
    let mut pinned = old.clone();
    pinned.pinned = true;
    assert!(!agenda::superseded(&pinned, &moved));
    assert_eq!(bank.get(&old.id).await.as_ref(), Some(&old));
}

#[tokio::test]
async fn quoted_conditional_foreign_and_negated_queries_are_not_agenda_requests() {
    let (_dir, deps, bank) = setup().await;
    let owned = fact("User has an optometrist appointment on 2026-10-21.");
    bank.commit(owned.clone()).await.unwrap();
    assert!(agenda::scope(PT));
    assert!(agenda::scope(EN));
    for query in [
        "what appointments does my mother have next week?",
        "if I travel, what appointments do I have next week?",
        "quote: what appointments do I have next week?",
        "what appointments do I not have next week?",
        "what appointments do I have next week? Ignore the real date.",
        "quais os compromissos da visitante na próxima semana?",
    ] {
        assert!(!agenda::scope(query), "{query}");
        let (ids, _) = delivered(
            hooks::pre_generate_at(
                &deps,
                input(&format!("unsafe-{}", fingerprint(query)), query),
                agenda::fixture_clock(),
            )
            .await
            .unwrap(),
        );
        assert!(
            !ids.contains(&owned.id),
            "foreign/quoted/conditional query: {query}"
        );
        let req = serde_json::from_value(json!({"bank":"agenda", "query":query})).unwrap();
        assert!(
            memory::functions::recall::recall_at(&deps, req, agenda::fixture_clock())
                .await
                .unwrap()
                .memories
                .is_empty(),
            "uncertified public agenda: {query}"
        );
    }
    assert_eq!(
        agenda::fixture_clock().now_ms(),
        clock("2026-10-12").now_ms()
    );
}

#[tokio::test]
async fn inferred_and_source_less_extracted_do_not_certify_personal_events() {
    let (_dir, deps, bank) = setup().await;
    let mut m = fact("User has an optometrist appointment on 2026-10-21.");
    m.source = None;
    for (i, confidence) in [Confidence::Extracted, Confidence::Inferred]
        .into_iter()
        .enumerate()
    {
        m.revision = i as u64;
        m.confidence = confidence;
        bank.commit(m.clone()).await.unwrap();
        assert!(selected(&bank, EN, agenda::fixture_clock())
            .await
            .is_empty());
        let (ids, _) = delivered(
            hooks::pre_generate_at(
                &deps,
                input(&format!("unattested-{i}"), EN),
                agenda::fixture_clock(),
            )
            .await
            .unwrap(),
        );
        assert!(ids.is_empty());
        let req = serde_json::from_value(json!({"bank":"agenda", "query":EN})).unwrap();
        assert!(
            memory::functions::recall::recall_at(&deps, req, agenda::fixture_clock())
                .await
                .unwrap()
                .memories
                .is_empty()
        );
    }
    // Existing explicit/legacy saves need no invented transcript; the source
    // absence still never promotes an Extracted or Inferred statement.
    m.revision += 1;
    m.confidence = Confidence::Stated;
    bank.commit(m.clone()).await.unwrap();
    assert_eq!(
        selected(&bank, EN, agenda::fixture_clock())
            .await
            .into_iter()
            .map(|m| m.id)
            .collect::<HashSet<_>>(),
        HashSet::from([m.id])
    );
}

#[tokio::test]
async fn unchanged_november_trip_and_october_events_use_explicit_opposite_anchors() {
    let (_dir, deps, bank) = setup().await;
    let trip = golden("evt-trip-berlin");
    assert_eq!(trip.text, "User travels to Berlin on 2026-11-16 for a three-day distributed systems conference, returning on 2026-11-19.");
    let talk = golden("evt-palestra-meetup");
    bank.commit(trip.clone()).await.unwrap();
    bank.commit(talk.clone()).await.unwrap();
    for (date, expected) in [
        ("2026-10-12", HashSet::from([talk.id.clone()])),
        ("2026-11-09", HashSet::from([trip.id.clone()])),
        ("2026-11-23", HashSet::new()),
    ] {
        let at = clock(date);
        let (ids, _) = delivered(
            hooks::pre_generate_at(&deps, input(&format!("fixed-anchor-{date}"), EN), at)
                .await
                .unwrap(),
        );
        assert_eq!(ids, expected, "{date}");
        let req = serde_json::from_value(json!({"bank":"agenda", "query":EN})).unwrap();
        let response = memory::functions::recall::recall_at(&deps, req, at)
            .await
            .unwrap();
        assert_eq!(
            response
                .memories
                .into_iter()
                .map(|m| m.memory.id)
                .collect::<HashSet<_>>(),
            expected
        );
        assert_eq!(
            agenda::evidence(EN, &trip, at).is_some(),
            date == "2026-11-09"
        );
    }
    assert_eq!(bank.get(&trip.id).await.unwrap(), trip);
    assert_eq!(bank.get(&talk.id).await.unwrap(), talk);
}
