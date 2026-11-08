// Consumer regressions run the real pre_generate hook against temporary banks.
// The producer envelope below is a consumer boundary fixture, not proof of
// delivery-channel authority. Producer behavioral tests are separate.
// Synthetic controls use the production lexical fallback. Official target
// controls additionally replay genuine cached vectors; no embedder is run.
use super::*;

#[path = "neutral_constraints.rs"]
mod neutral_constraints;

#[path = "identity_relations.rs"]
mod identity_relations;

#[path = "life_relations.rs"]
mod life_relations;

#[tokio::test]
async fn directive_relation_original_role_pairs_keep_each_positive_and_reject_each_isolated_extra()
{
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (query_id, required, incidental) in [
        ("q-evt-idioma", "evt-pref-idioma", "id-idiomas"),
        ("q-evt-time-format", "evt-pref-24h", "evt-passport"),
        ("q-evt-time-format", "evt-pref-24h", "en-work-deploy-window"),
        (
            "q-evt-code-lang",
            "evt-pref-code-english",
            "evt-pref-idioma",
        ),
    ] {
        let original = corpus["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == query_id)
            .unwrap();
        assert_eq!(original["expect"], json!([required]));
        let query = original["text"].as_str().unwrap();
        for (id, expected_support) in [(required, true), (incidental, false)] {
            let fact = corpus["memories"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["key"] == id)
                .unwrap();
            let text = fact["text"].as_str().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            bank.commit(Memory {
                id: id.into(),
                text: text.into(),
                entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
                tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
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
            })
            .await
            .unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            let supported = reasons.get(id).is_some_and(|reason| reason.supported());
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-original-isolated","stage":"before_select",
                    "query_id":query_id,"query":query,"independent_expected_support":expected_support,
                    "candidate_evidence":directive_candidate_audit(&ranked,&evidence,&[id])
                })
            );
            if supported != expected_support {
                failures.push(format!(
                    "{query_id}/{id}: certificado {supported} != {expected_support}"
                ));
            }
            let session = format!("directive-original-isolated-{executed}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            let expected = if expected_support {
                json!([id])
            } else {
                json!([])
            };
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-original-isolated","stage":"hook_result",
                    "query_id":query_id,"candidate_id":id,"independent_expected":expected,
                    "selected":ids(&response)
                })
            );
            if ids(&response) != expected {
                failures.push(format!("{query_id}/{id}: seleção isolada: {response}"));
            }
            rules_preserved(&response);
            executed += 1;
        }
    }
    assert_eq!(executed, 8);
    assert!(
        failures.is_empty(),
        "original directive role pairs: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_isolated_domains_do_not_certify_mentions_or_foreign_targets() {
    let cases = [
        (
            "In which language should you reply to me?",
            "User wants the assistant to reply in Catalan.",
            &[
                "User does not want the assistant to reply in Catalan.",
                "User quotes: \"The assistant should reply in Catalan.\"",
                "The archivist reports that a visitor wants the assistant to reply in Catalan.",
                "The visitor wants the assistant to reply in Catalan.",
                "If the visitor arrives, the assistant would reply in Catalan.",
                "The rehearsal mentions an assistant reply in Catalan as an example.",
                "The label reads assistant reply Catalan; its role is unspecified.",
            ][..],
        ),
        (
            "Em que língua você deve me responder?",
            "A usuária quer que o assistente responda em italiano.",
            &[
                "A usuária não quer que o assistente responda em italiano.",
                "A usuária cita: \"O assistente deve responder em italiano.\"",
                "A arquivista relata que uma visitante quer que o assistente responda em italiano.",
                "A visitante quer que o assistente responda em italiano.",
                "Se a visitante chegar, o assistente poderia responder em italiano.",
                "O ensaio menciona uma resposta do assistente em italiano como exemplo.",
                "O rótulo contém assistente responder italiano; seu papel é desconhecido.",
            ][..],
        ),
        (
            "Should you write times for me as 11:45 or as 11:45 AM?",
            "User wants times in 12-hour format, never 24-hour format.",
            &[
                "User does not want times in 12-hour format.",
                "User quotes: \"Write times in 12-hour format.\"",
                "The archivist reports that a visitor wants times in 12-hour format.",
                "The visitor wants times in 12-hour format.",
                "If the visitor arrives, times could be written in 12-hour format.",
                "The rehearsal mentions writing times in 12-hour format as an example.",
                "The label reads write times 11:45 AM; its role is unspecified.",
            ][..],
        ),
        (
            "Você deve escrever os horários para mim como 11:45 ou como 11:45 AM?",
            "A usuária quer horários no formato de 12 horas, nunca no formato de 24 horas.",
            &[
                "A usuária não quer horários no formato de 12 horas.",
                "A usuária cita: \"Escreva horários no formato de 12 horas.\"",
                "A arquivista relata que uma visitante quer horários no formato de 12 horas.",
                "A visitante quer horários no formato de 12 horas.",
                "Se a visitante chegar, horários poderiam ser escritos no formato de 12 horas.",
                "O ensaio menciona escrever horários no formato de 12 horas como exemplo.",
                "O rótulo contém escrever horários 11:45 AM; seu papel é desconhecido.",
            ][..],
        ),
        (
            "For code written by you, in which language should variable names and comments be?",
            "User wants code written by the assistant, including variable names and comments, in Catalan.",
            &[
                "User does not want code written by the assistant, including variable names and comments, in Catalan.",
                "User quotes: \"Code written by the assistant, including variable names and comments, should be in Catalan.\"",
                "The archivist reports that a visitor wants code written by the assistant, including variable names and comments, in Catalan.",
                "The visitor wants code written by the assistant, including variable names and comments, in Catalan.",
                "If the visitor arrives, code written by the assistant, including variable names and comments, could be in Catalan.",
                "The rehearsal mentions code written by the assistant, including variable names and comments, in Catalan as an example.",
                "The label reads code variable names comments Catalan; its role is unspecified.",
            ][..],
        ),
        (
            "No código escrito por você, os nomes de variáveis e os comentários devem ficar em que idioma?",
            "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
            &[
                "A usuária não pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
                "A usuária cita: \"O código escrito pelo assistente, incluindo nomes de variáveis e comentários, deve ficar em italiano.\"",
                "A arquivista relata que uma visitante pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
                "A visitante pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
                "Se a visitante chegar, o código escrito pelo assistente, incluindo nomes de variáveis e comentários, poderia ficar em italiano.",
                "O ensaio menciona código escrito pelo assistente, incluindo nomes de variáveis e comentários, em italiano como exemplo.",
                "O rótulo contém código nomes de variáveis comentários italiano; seu papel é desconhecido.",
            ][..],
        ),
    ];
    let domains = [
        "negative",
        "quoted",
        "reported",
        "foreign",
        "conditional",
        "example",
        "unknown",
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (g, (query, positive, negatives)) in cases.into_iter().enumerate() {
        assert_eq!(negatives.len(), domains.len());
        for (d, text, expected_support) in std::iter::once(("asserted", positive, true)).chain(
            domains
                .into_iter()
                .zip(negatives.iter().copied())
                .map(|(domain, text)| (domain, text, false)),
        ) {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            bank.commit(Memory {
                id: "candidate".into(),
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
            })
            .await
            .unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            let supported = reasons
                .get("candidate")
                .is_some_and(|reason| reason.supported());
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-domain","stage":"before_select",
                    "query":query,"text":text,"independent_domain":d,
                    "independent_expected_support":expected_support,
                    "candidate_evidence":directive_candidate_audit(&ranked,&evidence,&["candidate"])
                })
            );
            if supported != expected_support {
                failures.push(format!(
                    "{g}/{d}: certificado {supported} != {expected_support}"
                ));
            }
            let session = format!("directive-domain-{g}-{d}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            let expected = if expected_support {
                json!(["candidate"])
            } else {
                json!([])
            };
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-domain","stage":"hook_result",
                    "query":query,"independent_domain":d,"independent_expected":expected,
                    "selected":ids(&response)
                })
            );
            if ids(&response) != expected {
                failures.push(format!("{g}/{d}: seleção isolada: {response}"));
            }
            rules_preserved(&response);
            executed += 1;
        }
    }
    assert_eq!(executed, 48);
    assert!(
        failures.is_empty(),
        "directive assertion domains: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_inverse_pairs_keep_response_ability_format_event_and_code_prose_distinct(
) {
    let groups = [
        (
            "User wants the assistant to reply in Catalan.",
            "User speaks Catalan fluently.",
            "In which language should you reply to me?",
            "Do I speak Catalan fluently?",
        ),
        (
            "A usuária quer que o assistente responda em italiano.",
            "A usuária fala italiano fluentemente.",
            "Em que língua você deve me responder?",
            "Eu falo italiano fluentemente?",
        ),
        (
            "User wants times in 12-hour format, never 24-hour format.",
            "User has a pottery appointment at 11:45.",
            "Should you write times for me as 11:45 or as 11:45 AM?",
            "At what time is my pottery appointment?",
        ),
        (
            "A usuária quer horários no formato de 12 horas, nunca no formato de 24 horas.",
            "A usuária tem um compromisso de cerâmica às 11:45.",
            "Você deve escrever os horários para mim como 11:45 ou como 11:45 AM?",
            "Em que horário é meu compromisso de cerâmica?",
        ),
        (
            "User wants code written by the assistant, including variable names and comments, in Catalan.",
            "User wants prose written by the assistant, including variable names and comments, in Italian.",
            "For code written by you, in which language should variable names and comments be?",
            "For prose written by you, in which language should variable names and comments be?",
        ),
        (
            "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
            "A usuária pede que a prosa escrita pelo assistente, incluindo nomes de variáveis e comentários, fique em catalão.",
            "No código escrito por você, os nomes de variáveis e os comentários devem ficar em que idioma?",
            "Na prosa escrita por você, os nomes de variáveis e os comentários devem ficar em que idioma?",
        ),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (g, (directed, other_role, directed_query, other_query)) in groups.into_iter().enumerate() {
        for (q, (query, expected)) in [(directed_query, "directed"), (other_query, "other-role")]
            .into_iter()
            .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            for (id, text) in [("directed", directed), ("other-role", other_role)] {
                bank.commit(Memory {
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
                })
                .await
                .unwrap();
            }
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            let forbidden = if expected == "directed" {
                "other-role"
            } else {
                "directed"
            };
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-inverse","stage":"before_select",
                    "query":query,"independent_required":[expected],"independent_forbidden":[forbidden],
                    "candidate_evidence":directive_candidate_audit(&ranked,&evidence,&["directed","other-role"])
                })
            );
            if !reasons
                .get(expected)
                .is_some_and(|reason| reason.supported())
            {
                failures.push(format!("{g}/{q}: positivo inverso sem prova: {expected}"));
            }
            if reasons
                .get(forbidden)
                .is_some_and(|reason| reason.supported())
            {
                failures.push(format!("{g}/{q}: papel diverso certificado: {forbidden}"));
            }
            let session = format!("directive-inverse-{g}-{q}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-inverse","stage":"hook_result",
                    "query":query,"independent_expected":[expected],"selected":ids(&response)
                })
            );
            if ids(&response) != json!([expected]) {
                failures.push(format!("{g}/{q}: seleção inversa: {response}"));
            }
            rules_preserved(&response);
            executed += 1;
        }
    }
    assert_eq!(executed, 12);
    for (query, positive, foreign) in [
        (
            "Eu desenho mapas diariamente?",
            "A usuária desenha mapas diariamente.",
            "A visitante desenha mapas diariamente.",
        ),
        (
            "Eu canto melodias suavemente?",
            "O usuário canta melodias suavemente.",
            "O visitante canta melodias suavemente.",
        ),
    ] {
        failures.extend(
            directive_review_run(
                "personal-inflection",
                query,
                &[("positive", positive), ("foreign", foreign)],
                &["positive"],
            )
            .await,
        );
    }
    assert!(failures.is_empty(), "directive inverse roles: {failures:?}");
}

fn directive_candidate_audit(
    ranked: &[(Memory, f32)],
    evidence: &HashMap<String, Evidence>,
    examined: &[&str],
) -> Value {
    let reasons = admission_reasons(ranked, evidence);
    json!(examined
        .iter()
        .map(|id| {
            let e = evidence.get(*id);
            json!({
                "id":id,
                "text":ranked.iter().find(|(m,_)|m.id==*id).map(|(m,_)|&m.text),
                "candidate_present":ranked.iter().any(|(m,_)|m.id==*id),
                "supported":reasons.get(*id).is_some_and(|reason|reason.supported()),
                "reason":reasons.get(*id),
                "matched":e.map(|e|&e.matched),
                "vector":e.map(|e|e.vector),
                "target":e.map(|e|json!({
                    "required":e.target_scope_required,"matched":e.target_scope_matched,
                    "kind":e.target_scope_kind,"span":e.target_scope_span,
                    "terms":e.target_scope_terms,"predicate":e.target_scope_predicate,
                    "subject":e.target_scope_subject,"object":e.target_scope_object,
                    "relative":e.target_scope_relative,"obligation":e.target_obligation,
                    "directive_required":e.directive_scope_required,
                    "directive_matched":e.directive_scope_matched,
                    "directive":e.target_directive
                })),
                "personal":e.map(|e|json!({
                    "required":e.personal_scope_required,"matched":e.personal_scope_matched,
                    "spans":e.personal_scope_spans,"terms":e.personal_scope_terms,
                    "predicates":e.personal_scope_predicates,"owners":e.personal_scope_owners
                }))
            })
        })
        .collect::<Vec<_>>())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directive_relation_official_positives_exclude_incidental_roles_in_all_cache_modes() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
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
            .map(|raw| serde_json::from_str(raw).unwrap())
            .unwrap_or_else(|| json!({"items":{},"model":""}));
        let dir = tempfile::tempdir().unwrap();
        let boundary = super::cached_boundary::boundary(dir.path(), cache.clone()).await;
        let deps = &boundary.deps;
        let mut cfg = (**deps.config.read().await).clone();
        cfg.embeddings_enabled = raw.is_some();
        *deps.config.write().await = Arc::new(cfg);
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for fact in corpus["memories"].as_array().unwrap() {
            let id = fact["key"].as_str().unwrap();
            let text = fact["text"].as_str().unwrap();
            let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
            bank.commit(Memory {
                id: id.into(),
                text: text.into(),
                entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
                tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
                confidence: crate::types::Confidence::Stated,
                corroboration: fact["corroboration"].as_u64().unwrap_or(0) as u32,
                pinned: fact["pinned"].as_bool().unwrap_or(false),
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
            if raw.is_some() {
                assert_eq!(cache["items"][id]["text"], text);
                let vector: Vec<f32> =
                    serde_json::from_value(cache["items"][id]["v"].clone()).unwrap();
                assert_eq!(vector.len(), cache["dims"].as_u64().unwrap() as usize);
                assert!(vector.iter().all(|n| n.is_finite()));
                bank.set_vector(id, cache["model"].as_str().unwrap(), text, vector)
                    .await
                    .unwrap();
            }
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let mut expected_calls = Vec::new();
        for (n, (query_id, required, forbidden)) in [
            ("q-evt-idioma", "evt-pref-idioma", &["id-idiomas"][..]),
            (
                "q-evt-time-format",
                "evt-pref-24h",
                &["evt-passport", "en-work-deploy-window"][..],
            ),
            (
                "q-evt-code-lang",
                "evt-pref-code-english",
                &["evt-pref-idioma"][..],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let original = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap();
            assert_eq!(original["expect"], json!([required]));
            let query = original["text"].as_str().unwrap();
            let vector: Option<Vec<f32>> = if raw.is_some() {
                assert_eq!(cache["items"][query_id]["text"], query);
                expected_calls.push(query.to_owned());
                let v: Vec<f32> =
                    serde_json::from_value(cache["items"][query_id]["v"].clone()).unwrap();
                assert_eq!(v.len(), cache["dims"].as_u64().unwrap() as usize);
                assert!(v.iter().all(|n| n.is_finite()));
                Some(v)
            } else {
                None
            };
            let (ranked, evidence) = bank
                .recall_evidence(
                    query,
                    vector.as_deref(),
                    cache["model"].as_str().unwrap_or(""),
                    usize::MAX,
                    30,
                    false,
                    0.0,
                )
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            let examined = std::iter::once(required)
                .chain(forbidden.iter().copied())
                .collect::<Vec<_>>();
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-official","stage":"before_select",
                    "lane":lane,"query_id":query_id,"query":query,
                    "independent_required":[required],"independent_forbidden":forbidden,
                    "candidate_evidence":directive_candidate_audit(&ranked,&evidence,&examined)
                })
            );
            if !reasons
                .get(required)
                .is_some_and(|reason| reason.supported())
            {
                failures.push(format!(
                    "{lane}/{query_id}: positivo original sem prova: {required}"
                ));
            }
            for id in forbidden {
                if reasons.get(*id).is_some_and(|reason| reason.supported()) {
                    failures.push(format!("{lane}/{query_id}: certificado incidental: {id}"));
                }
            }
            let session = format!("directive-official-{lane}-{n}");
            let response = hook(
                deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            println!(
                "{}",
                json!({
                    "diagnostic":"directive-relation-official","stage":"hook_result",
                    "lane":lane,"query_id":query_id,"independent_expected":[required],
                    "selected":ids(&response)
                })
            );
            if ids(&response) != json!([required]) {
                failures.push(format!("{lane}/{query_id}: seleção exata: {response}"));
            }
            rules_preserved(&response);
            executed += 1;
        }
        assert_eq!(
            *boundary.calls.lock().unwrap(),
            expected_calls,
            "hybrid queries must use the genuine exact cache boundary"
        );
    }
    assert_eq!(executed, 9);
    assert!(
        failures.is_empty(),
        "directive roles, official corpus: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_output_target_and_witnesses_are_bound_before_select() {
    for (language, query, positive, foreign, source, governor) in [
        (
            "en",
            "In which language should you reply to me?",
            "User wants the assistant to reply in Finnish.",
            "User wants the translator to reply in Finnish.",
            vec!["user"],
            vec!["wants"],
        ),
        (
            "pt",
            "Em que língua você deve me responder?",
            "A usuária quer que o assistente responda em finlandês.",
            "A usuária quer que o tradutor responda em finlandês.",
            vec!["a", "usuária"],
            vec!["quer"],
        ),
    ] {
        for (domain, text, expected_support) in [
            ("asserted", positive.to_owned(), true),
            ("foreign-target", foreign.to_owned(), false),
            (
                "interrogative",
                format!("{}?", positive.trim_end_matches('.')),
                false,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            bank.commit(Memory {
                id: "candidate".into(),
                text: text.clone(),
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
            })
            .await
            .unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            assert_eq!(
                reasons
                    .get("candidate")
                    .is_some_and(|reason| reason.supported()),
                expected_support,
                "{language}/{domain}: {}",
                directive_candidate_audit(&ranked, &evidence, &["candidate"])
            );
            let e = &evidence["candidate"];
            assert!(e.directive_scope_required);
            assert_eq!(e.directive_scope_matched, expected_support);
            if expected_support {
                let audit = e.target_directive.as_ref().unwrap();
                let tokens = crate::index::tokenize(&text);
                let witnessed = |field: &str| {
                    let span = audit[field].as_array().unwrap();
                    tokens[span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize]
                        .to_vec()
                };
                assert_eq!(witnessed("source_span"), source);
                assert_eq!(witnessed("governor_span"), governor);
                assert!(!witnessed("assistant_span").is_empty());
                assert!(!witnessed("argument_span").is_empty());
                assert_eq!(witnessed("value_span").len(), 1);
                assert_eq!(audit["domain"], "asserted");
                assert_eq!(audit["polarity"], "affirmative");
            }
            let session = format!("directive-target-{language}-{domain}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected_support {
                    json!(["candidate"])
                } else {
                    json!([])
                }
            );
            rules_preserved(&response);
        }
    }
}

#[tokio::test]
async fn obligation_relation_query_punctuation_preserves_isolated_hook_certificates() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for (language, query, candidate, object, target, governor, span, argument, boundary) in [
        (
            "en",
            "How many clips does a parcel need before transit",
            "Each parcel needs two clips before transit",
            "clips",
            "parcel",
            "need",
            [2, 9],
            [3, 5],
            [5, 7],
        ),
        (
            "pt",
            "Quantas etiquetas a bandeja precisa antes do transporte",
            "Cada bandeja precisa de duas etiquetas antes do transporte",
            "etiquetas",
            "bandeja",
            "precisa",
            [1, 8],
            [4, 6],
            [6, 9],
        ),
    ] {
        for query_suffix in ["", "?"] {
            let query = format!("{query}{query_suffix}");
            for (candidate_suffix, domain, matched, expected_ids) in [
                ("", "asserted", true, json!(["candidate"])),
                ("?", "interrogative", false, json!([])),
            ] {
                // One candidate per temporary bank prevents deduplication from
                // masking an unsupported certificate or a lost positive.
                let candidate = format!("{candidate}{candidate_suffix}");
                let dir = tempfile::tempdir().unwrap();
                let deps = offline(dir.path());
                let store = deps.store().await;
                let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
                bank.commit(Memory {
                    id: "candidate".into(),
                    text: candidate.clone(),
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
                })
                .await
                .unwrap();
                bank.set_rule("standing", "Preserve the permanent safety rule.")
                    .unwrap();
                let (candidates, evidence) = bank
                    .recall_evidence(&query, None, "", usize::MAX, 30, false, 0.0)
                    .await;
                let e = evidence.get("candidate");
                let expected = json!({
                    "candidate_count":1, "candidate_present":true, "vector":false,
                    "target_scope_required":true, "target_scope_matched":matched,
                    "kind":"Quantity", "span":span, "terms":[object],
                    "predicate":governor, "subject":[target], "object":[], "relative":[],
                    "obligation":{
                        "query_governor":governor, "query_span":span,
                        "target":[target], "object":[object], "matched":matched,
                        "assertion":{
                            "governor_span":[2, 3], "source_span":[1, 2],
                            "argument_span":argument, "target_span":[1, 2],
                            "boundary_span":boundary, "frame":"universal_bearer",
                            "domain":domain, "polarity":"affirmative"
                        }
                    }
                });
                let result = json!({
                    "candidate_count":candidates.len(),
                    "candidate_present":candidates.iter().any(|(memory, _)| memory.id == "candidate"),
                    "vector":e.map(|e| e.vector),
                    "target_scope_required":e.map(|e| e.target_scope_required),
                    "target_scope_matched":e.map(|e| e.target_scope_matched),
                    "kind":e.and_then(|e| e.target_scope_kind.as_ref()),
                    "span":e.and_then(|e| e.target_scope_span),
                    "terms":e.map(|e| &e.target_scope_terms),
                    "predicate":e.and_then(|e| e.target_scope_predicate.as_ref()),
                    "subject":e.map(|e| &e.target_scope_subject),
                    "object":e.map(|e| &e.target_scope_object),
                    "relative":e.map(|e| &e.target_scope_relative),
                    "obligation":e.and_then(|e| e.target_obligation.as_ref())
                });
                println!(
                    "{}",
                    json!({
                        "diagnostic":"query-candidate-domain-separation", "stage":"before_select",
                        "language":language, "query":query, "candidate":candidate,
                        "expected":expected, "result":result
                    })
                );
                if result != expected {
                    failures.push(format!("{language}/{query_suffix}/{candidate_suffix}/pré-seleção: {result} != {expected}"));
                }
                let session = "query-candidate-domain-separation";
                let response = hook(
                    &deps,
                    session,
                    json!([{"role":"user","content":query}]),
                    context(session, "direct", &query),
                )
                .await;
                let expected = json!({
                    "memory_ids":expected_ids, "decision":"continue", "standing_rule_preserved":true
                });
                let result = json!({
                    "memory_ids":ids(&response), "decision":response["decision"],
                    "standing_rule_preserved":response["mutations"]["system_prompt"]
                        .as_str().is_some_and(|prompt| prompt.contains("Preserve the permanent safety rule."))
                });
                println!(
                    "{}",
                    json!({
                        "diagnostic":"query-candidate-domain-separation", "stage":"hook_result",
                        "language":language, "query":query, "candidate":candidate,
                        "expected":expected, "result":result
                    })
                );
                if result != expected {
                    failures.push(format!(
                        "{language}/{query_suffix}/{candidate_suffix}/hook: {result} != {expected}"
                    ));
                }
                executed += 1;
            }
        }
    }
    assert_eq!(executed, 8);
    assert!(
        failures.is_empty(),
        "certificados isolados da consulta: {failures:?}"
    );
}

#[tokio::test]
async fn target_relation_review_r20_relative_antecedent_binding_hook() {
    let query = "Which service consumes the telemetry packets Vega publishes to Relay?";
    let answer =
        "Vega publishes telemetry packets to Relay topics that the archive service consumes.";
    let incidental = "Vega publishes telemetry packets to Relay topics alongside the billing records that the archive service consumes.";
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, fact) in [("answer", answer), ("incidental", incidental)] {
        bank.commit(Memory {
            id: id.into(),
            text: fact.into(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let (candidates, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    let mut failures = Vec::new();
    let mut executed = 0;
    for (id, fact, expected_matched) in
        [("answer", answer, true), ("incidental", incidental, false)]
    {
        let e = evidence.get(id);
        let expected = json!({
            "candidate_present":true, "target_scope_required":true,
            "target_scope_matched":expected_matched, "vector":false
        });
        let result = json!({
            "candidate_present":candidates.iter().any(|(m,_)| m.id == id),
            "target_scope_required":e.map(|e| e.target_scope_required),
            "target_scope_matched":e.map(|e| e.target_scope_matched),
            "vector":e.map(|e| e.vector)
        });
        println!(
            "{}",
            json!({
                "diagnostic":"relative-antecedent-binding", "stage":"before_select",
                "query":query, "id":id, "fact":fact, "expected":expected, "result":result,
                "witness":e.map(|e| json!({"kind":e.target_scope_kind,"span":e.target_scope_span,
                    "terms":e.target_scope_terms,"predicate":e.target_scope_predicate,
                    "subject":e.target_scope_subject,"object":e.target_scope_object,
                    "relative":e.target_scope_relative}))
            })
        );
        executed += 1;
        if result != expected {
            failures.push(format!(
                "pré-seleção/{id}: esperado={expected}, obtido={result}"
            ));
        }
    }
    let session = "relative-antecedent-binding";
    let response = hook(
        &deps,
        session,
        json!([{"role":"user","content":query}]),
        context(session, "direct", query),
    )
    .await;
    let expected = json!({
        "memory_ids":["answer"], "decision":"continue", "standing_rule_preserved":true
    });
    let result = json!({
        "memory_ids":ids(&response), "decision":response["decision"],
        "standing_rule_preserved":response["mutations"]["system_prompt"]
            .as_str().is_some_and(|prompt| prompt.contains("Preserve the permanent safety rule."))
    });
    println!(
        "{}",
        json!({
            "diagnostic":"relative-antecedent-binding", "stage":"hook_result", "query":query,
            "fact":{"answer":answer,"incidental":incidental}, "expected":expected,"result":result
        })
    );
    if result != expected {
        failures.push(format!(
            "hook: esperado={expected}, obtido={result}; resposta={response}"
        ));
    }
    assert_eq!(executed, 2);
    assert!(failures.is_empty(), "antecedente no hook: {failures:?}");
}

#[tokio::test]
async fn obligation_relation_review_r20_candidate_questions_not_assertions_hook() {
    let mut failures = Vec::new();
    let mut executed = 0;
    let mut hook_cases = 0;
    for (language, query, answer, incidental) in [
        (
            "en",
            "How many seals does a crate need before dispatch?",
            "Each crate needs three seals before dispatch.",
            "Each crate needs three seals before dispatch?",
        ),
        (
            "pt",
            "Quantos lacres uma caixa precisa antes do envio?",
            "Cada caixa precisa de três lacres antes do envio.",
            "Cada caixa precisa de três lacres antes do envio?",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for (id, fact) in [("answer", answer), ("incidental", incidental)] {
            bank.commit(Memory {
                id: id.into(),
                text: fact.into(),
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
            })
            .await
            .unwrap();
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let (candidates, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        for (id, fact, expected_matched) in
            [("answer", answer, true), ("incidental", incidental, false)]
        {
            let e = evidence.get(id);
            let audit = e.and_then(|e| e.target_obligation.as_ref());
            let expected = json!({
                "candidate_present":true, "target_scope_required":true,
                "target_scope_matched":expected_matched, "vector":false,
                "obligation_matched":expected_matched, "domain_asserted":expected_matched
            });
            let result = json!({
                "candidate_present":candidates.iter().any(|(m,_)| m.id == id),
                "target_scope_required":e.map(|e| e.target_scope_required),
                "target_scope_matched":e.map(|e| e.target_scope_matched),
                "vector":e.map(|e| e.vector),
                "obligation_matched":audit.and_then(|audit| audit["matched"].as_bool()),
                "domain_asserted":audit.and_then(|audit| audit["assertion"]["domain"].as_str())
                    .map(|domain| domain == "asserted")
            });
            println!(
                "{}",
                json!({
                    "diagnostic":"candidate-questions-not-assertions", "stage":"before_select",
                    "language":language, "query":query, "id":id, "fact":fact,
                    "expected":expected, "result":result,
                    "witness":e.map(|e| json!({"kind":e.target_scope_kind,"span":e.target_scope_span,
                        "terms":e.target_scope_terms,"predicate":e.target_scope_predicate,
                        "subject":e.target_scope_subject,"object":e.target_scope_object,
                        "relative":e.target_scope_relative,"obligation":e.target_obligation}))
                })
            );
            executed += 1;
            if result != expected {
                failures.push(format!(
                    "{language}/pré-seleção/{id}: esperado={expected}, obtido={result}"
                ));
            }
        }
        let session = format!("candidate-question-{language}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let expected = json!({
            "memory_ids":["answer"], "decision":"continue", "standing_rule_preserved":true
        });
        let result = json!({
            "memory_ids":ids(&response), "decision":response["decision"],
            "standing_rule_preserved":response["mutations"]["system_prompt"]
                .as_str().is_some_and(|prompt| prompt.contains("Preserve the permanent safety rule."))
        });
        println!(
            "{}",
            json!({
                "diagnostic":"candidate-questions-not-assertions", "stage":"hook_result",
                "language":language, "query":query, "fact":{"answer":answer,"incidental":incidental},
                "expected":expected, "result":result
            })
        );
        hook_cases += 1;
        if result != expected {
            failures.push(format!(
                "{language}/hook: esperado={expected}, obtido={result}; resposta={response}"
            ));
        }
    }
    assert_eq!(executed, 4);
    assert_eq!(hook_cases, 2);
    assert!(failures.is_empty(), "perguntas no hook: {failures:?}");
}

#[tokio::test]
async fn target_relation_isolated_certificates_do_not_depend_on_deduplication() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for (case, query, answer, incidental, obligation) in [
        (
            "relative",
            "Which service consumes the telemetry packets Vega publishes to Relay?",
            "Vega publishes telemetry packets to Relay topics that the archive service consumes.",
            "Vega publishes telemetry packets to Relay topics alongside the billing records that the archive service consumes.",
            false,
        ),
        (
            "obligation-en",
            "How many seals does a crate need before dispatch?",
            "Each crate needs three seals before dispatch.",
            "Each crate needs three seals before dispatch?",
            true,
        ),
        (
            "obligation-pt",
            "Quantos lacres uma caixa precisa antes do envio?",
            "Cada caixa precisa de três lacres antes do envio.",
            "Cada caixa precisa de três lacres antes do envio?",
            true,
        ),
    ] {
        for (id, fact, expected_matched, expected_ids, domain) in [
            ("answer", answer, true, json!(["answer"]), "asserted"),
            ("incidental", incidental, false, json!([]), "interrogative"),
        ] {
            // Each local bank contains exactly one candidate. A positive cannot
            // conceal a false certificate through ordering or deduplication.
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let (foreign, _) = store.ensure_bank("foreign-isolated", None).await.unwrap();
            for (owner, memory_id, text, pinned) in [
                (&bank, id, fact, false),
                (&foreign, "foreign-answer", answer, true),
            ] {
                owner.commit(Memory {
                    id: memory_id.into(),
                    text: text.into(),
                    entities: Vec::new(),
                    tags: Vec::new(),
                    confidence: crate::types::Confidence::Stated,
                    corroboration: 0,
                    pinned,
                    source: None,
                    created_at: now_ms(),
                    updated_at: now_ms(),
                    stated_at: None,
                    invalid_at: None,
                    superseded_by: None,
                    revision: 0,
                })
                .await
                .unwrap();
            }
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (candidates, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let e = evidence.get(id);
            let audit = e.and_then(|e| e.target_obligation.as_ref());
            let expected = json!({
                "candidate_present":true, "candidate_count":1,
                "target_scope_required":true, "target_scope_matched":expected_matched,
                "vector":false, "foreign_candidate_present":false,
                "obligation_matched":obligation.then_some(expected_matched),
                "domain":obligation.then_some(domain)
            });
            let result = json!({
                "candidate_present":candidates.iter().any(|(m,_)| m.id == id),
                "candidate_count":candidates.len(),
                "target_scope_required":e.map(|e| e.target_scope_required),
                "target_scope_matched":e.map(|e| e.target_scope_matched),
                "vector":e.map(|e| e.vector),
                "foreign_candidate_present":candidates.iter().any(|(m,_)| m.id == "foreign-answer"),
                "obligation_matched":audit.and_then(|audit| audit["matched"].as_bool()),
                "domain":audit.and_then(|audit| audit["assertion"]["domain"].as_str())
            });
            println!("{}", json!({
                "diagnostic":"isolated-relation-certificate", "stage":"before_select",
                "case":case, "query":query, "id":id, "fact":fact,
                "expected":expected, "result":result,
                "witness":e.map(|e| json!({"kind":e.target_scope_kind,
                    "span":e.target_scope_span, "predicate":e.target_scope_predicate,
                    "subject":e.target_scope_subject, "object":e.target_scope_object,
                    "relative":e.target_scope_relative, "obligation":e.target_obligation}))
            }));
            if result != expected {
                failures.push(format!("{case}/{id}/pré-seleção: {result} != {expected}"));
            }
            let session = format!("isolated-certificate-{case}-{id}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            let expected = json!({
                "memory_ids":expected_ids, "decision":"continue",
                "standing_rule_preserved":true
            });
            let result = json!({
                "memory_ids":ids(&response), "decision":response["decision"],
                "standing_rule_preserved":response["mutations"]["system_prompt"]
                    .as_str().is_some_and(|prompt| prompt.contains("Preserve the permanent safety rule."))
            });
            println!("{}", json!({
                "diagnostic":"isolated-relation-certificate", "stage":"hook_result",
                "case":case, "query":query, "id":id, "fact":fact,
                "expected":expected, "result":result
            }));
            if result != expected {
                failures.push(format!("{case}/{id}/hook: {result} != {expected}; {response}"));
            }
            executed += 1;
        }
    }
    assert_eq!(executed, 6);
    assert!(failures.is_empty(), "certificados isolados: {failures:?}");
}

// Paired controls use new lexical fixtures: only the grammatical role changes.
async fn slot_projection_fixture(dir: &std::path::Path) -> Arc<Deps> {
    let deps = offline(dir);
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, text, tags) in [
        ("pt-answer", "Eu reviso o rascunho em silêncio.", vec![]),
        (
            "pt-slot",
            "O intervalo da sessão é organizado pelo facilitador.",
            vec![],
        ),
        ("en-answer", "I review the draft after lunch.", vec![]),
        (
            "en-slot",
            "The portion of the cycle is tracked by another department.",
            vec![],
        ),
        ("pt-manager", "Meu gestor é a Renata.", vec!["gestor"]),
        ("en-manager", "My manager is Morgan.", vec!["manager"]),
        ("pt-schedule", "Meu horário é noturno.", vec!["horário"]),
        (
            "en-schedule",
            "My schedule uses evening hours.",
            vec!["hours", "schedule"],
        ),
    ] {
        bank.commit(Memory {
            id: id.into(),
            text: text.into(),
            entities: Vec::new(),
            tags: tags.into_iter().map(str::to_owned).collect(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    deps
}

#[tokio::test]
async fn slot_projection_pt_answer_slot_does_not_anchor_an_unrelated_line() {
    let dir = tempfile::tempdir().unwrap();
    let deps = slot_projection_fixture(dir.path()).await;
    let text = "Em que intervalo da sessão eu reviso o rascunho?";
    let response = hook(
        &deps,
        "slot-pt",
        json!([{"role":"user","content":text}]),
        context("slot-pt", "direct", text),
    )
    .await;
    println!(
        "{}",
        json!({"diagnostic":"slot-projection-paired","language":"pt","query":text,"selected":ids(&response)})
    );
    rules_preserved(&response);
    assert_eq!(
        ids(&response),
        json!(["pt-answer"]),
        "slot-only is not a relation: {response}"
    );
}

#[tokio::test]
async fn slot_projection_en_answer_slot_does_not_anchor_an_unrelated_line() {
    let dir = tempfile::tempdir().unwrap();
    let deps = slot_projection_fixture(dir.path()).await;
    let text = "During what portion of the cycle do I review the draft?";
    let response = hook(
        &deps,
        "slot-en",
        json!([{"role":"user","content":text}]),
        context("slot-en", "direct", text),
    )
    .await;
    println!(
        "{}",
        json!({"diagnostic":"slot-projection-paired","language":"en","query":text,"selected":ids(&response)})
    );
    rules_preserved(&response);
    assert_eq!(
        ids(&response),
        json!(["en-answer"]),
        "slot-only is not a relation: {response}"
    );
}

#[tokio::test]
async fn slot_projection_subject_attributes_and_ambiguous_fragments_keep_recall() {
    let dir = tempfile::tempdir().unwrap();
    let deps = slot_projection_fixture(dir.path()).await;
    for (n, (text, required)) in [
        ("Qual é o intervalo da sessão?", "pt-slot"),
        ("What is the portion of the cycle?", "en-slot"),
        ("Quem é meu gestor?", "pt-manager"),
        ("Who is my manager?", "en-manager"),
        ("Qual é meu horário?", "pt-schedule"),
        ("What hours are in my schedule?", "en-schedule"),
        ("Em que intervalo da sessão?", "pt-slot"),
        ("During what portion of the cycle?", "en-slot"),
    ]
    .iter()
    .enumerate()
    {
        let session = format!("slot-control-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        println!(
            "{}",
            json!({"diagnostic":"slot-projection-control","query":text,"selected":ids(&response)})
        );
        rules_preserved(&response);
        assert!(ids(&response).as_array().unwrap().contains(&json!(required)),
            "a subject, legitimate attribute or ambiguous fragment must retain prior recall: {text}: {response}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attribute_scope_layer_compounds_require_the_requested_subject() {
    let dir = tempfile::tempdir().unwrap();
    let boundary =
        super::cached_boundary::boundary(dir.path(), json!({"items":{},"model":""})).await;
    let deps = &boundary.deps;
    let mut cfg = (**deps.config.read().await).clone();
    cfg.embeddings_enabled = false;
    cfg.default_bank = "scope-global".into();
    *deps.config.write().await = Arc::new(cfg);
    let store = deps.store().await;
    let root_name = crate::scope::root_bank(dir.path()).unwrap();
    for (holder, id, text, tags) in [
        (
            root_name.clone(),
            "time-format",
            "I use 24-hour time format. Eu uso o formato de horário 24h.",
            vec!["format", "formato"],
        ),
        (
            crate::scope::agent_bank("scope-test"),
            "agent-format",
            "My agent uses opaline format with numbered steps. Meu agente usa o formato opalino.",
            vec![],
        ),
        (
            root_name.clone(),
            "report-format",
            "The report uses basalt format. O relatório usa o formato basalto.",
            vec!["format", "formato"],
        ),
        (
            root_name.clone(),
            "editor",
            "My code editor is Helix.",
            vec!["editor"],
        ),
        (
            root_name.clone(),
            "maintenance",
            "My maintenance is scheduled for Monday.",
            vec!["maintenance"],
        ),
        (
            root_name.clone(),
            "maintenance-action",
            "We maintain during the quiet interval.",
            vec![],
        ),
    ] {
        let (bank, _) = store.ensure_bank(&holder, None).await.unwrap();
        bank.commit(Memory {
            id: id.into(),
            text: text.into(),
            tags: tags.into_iter().map(str::to_owned).collect(),
            entities: Vec::new(),
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
        })
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
    }
    let (foreign, _) = store.ensure_bank("scope-foreign", None).await.unwrap();
    foreign
        .commit(Memory {
            id: "foreign-pinned".into(),
            text: "My agent uses opaline format. My code editor is Helix.".into(),
            entities: vec!["user".into()],
            tags: vec!["format".into(), "editor".into()],
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: true,
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            stated_at: None,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        })
        .await
        .unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (text, required, forbidden)) in [
        (
            "Which opaline format does my agent use?",
            Some("agent-format"),
            Some("time-format"),
        ),
        (
            "Qual opalino formato meu agente usa?",
            Some("agent-format"),
            Some("time-format"),
        ),
        (
            "Which basalt format does the report use?",
            Some("report-format"),
            Some("time-format"),
        ),
        (
            "Qual basalto formato o relatório usa?",
            Some("report-format"),
            Some("time-format"),
        ),
        (
            "Which unknownstone format does my agent use?",
            None,
            Some("time-format"),
        ),
        ("Which time format do I use?", Some("time-format"), None),
        ("Qual formato de horário eu uso?", Some("time-format"), None),
        (
            "Which code editor do I use?",
            Some("editor"),
            Some("time-format"),
        ),
        (
            "What maintenance?",
            Some("maintenance"),
            Some("time-format"),
        ),
        (
            "During what interval do we maintain?",
            Some("maintenance-action"),
            Some("time-format"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let session = format!("attribute-scope-{n}");
        boundary.sessions.lock().unwrap().insert(
            session.clone(),
            json!({"agent_profile":{"id":"scope-test"}}),
        );
        let input: PreGenerateInput = serde_json::from_value(json!({
            "session_id":session,"turn_id":format!("turn-{session}"),"step":0,
            "metadata":{"fs_scope":{"root":dir.path()}},
            "recall_context":context(&session,"direct",text),
            "generate":{"system_prompt":"offline scope control","messages":[{"role":"user","content":text}]}
        })).unwrap();
        let response = serde_json::to_value(pre_generate(deps, input).await.unwrap()).unwrap();
        executed += 1;
        let selected = ids(&response);
        let holders = response["annotations"]
            .get("memory_banks")
            .cloned()
            .unwrap_or_else(|| json!([]));
        println!(
            "{}",
            json!({"diagnostic":"attribute-scope-control","query":text,
            "selected":selected,"holders":holders,"expected_local_holder":root_name})
        );
        let admitted = selected.as_array().unwrap();
        if response["decision"] != "continue"
            || !response["mutations"]["system_prompt"]
                .as_str()
                .unwrap_or("")
                .contains("Preserve the permanent safety rule.")
        {
            failures.push(format!("{n}: permanent rule/decision: {response}"));
        }
        if let Some(id) = required {
            if let Some(position) = admitted.iter().position(|selected| selected == id) {
                let expected_holder = if id == "agent-format" {
                    crate::scope::agent_bank("scope-test")
                } else {
                    root_name.clone()
                };
                if holders[position] != expected_holder {
                    failures.push(format!("{n}: wrong holder: {response}"));
                }
            } else {
                failures.push(format!("{n}: missing {id}: {response}"));
            }
        }
        let exact = required.map_or_else(|| json!([]), |id| json!([id]));
        if selected != exact {
            failures.push(format!("{n}: exact selection {exact} required: {response}"));
        }
        if forbidden.is_some_and(|id| admitted.contains(&json!(id))) {
            failures.push(format!("{n}: unrelated attribute: {response}"));
        }
        if required.is_none() && !admitted.is_empty() {
            failures.push(format!("{n}: unknown subject admitted: {response}"));
        }
        if admitted.contains(&json!("foreign-pinned")) {
            failures.push(format!("{n}: foreign pinned leaked: {response}"));
        }
    }
    assert_eq!(executed, 10, "all paired controls must execute");
    assert!(failures.is_empty(), "attribute controls: {failures:?}");
    assert!(
        boundary.calls.lock().unwrap().is_empty(),
        "no embedding or unexpected RPC"
    );
}

#[tokio::test]
async fn genitive_attribute_scope_keeps_packaging_and_wall_colors_separate() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    let (foreign, _) = store.ensure_bank("genitive-foreign", None).await.unwrap();
    for (holder, id, text, pinned) in [
        (
            &bank,
            "packaging-color",
            "A cor de embalagem é azul. The packaging color is blue.",
            false,
        ),
        (
            &bank,
            "wall-color",
            "A cor de parede é verde. The wall color is green.",
            false,
        ),
        (
            &foreign,
            "foreign-pinned",
            "A cor de embalagem é dourada. The packaging color is gold.",
            true,
        ),
    ] {
        holder
            .commit(Memory {
                id: id.into(),
                text: text.into(),
                tags: vec!["cor".into(), "color".into()],
                entities: Vec::new(),
                confidence: crate::types::Confidence::Stated,
                corroboration: 0,
                pinned,
                source: None,
                created_at: now_ms(),
                updated_at: now_ms(),
                stated_at: None,
                invalid_at: None,
                superseded_by: None,
                revision: 0,
            })
            .await
            .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query, required)) in [
        ("Qual cor de embalagem eu uso?", Some("packaging-color")),
        ("Qual cor de parede eu uso?", Some("wall-color")),
        ("Qual cor de telhado eu uso?", None),
        (
            "Which color of the packaging do I use?",
            Some("packaging-color"),
        ),
        ("Which color of the wall do I use?", Some("wall-color")),
        ("Which color of the roof do I use?", None),
        ("Which packaging color do I use?", Some("packaging-color")),
        ("Which wall color do I use?", Some("wall-color")),
    ]
    .into_iter()
    .enumerate()
    {
        let session = format!("genitive-attribute-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        executed += 1;
        let selected = ids(&response);
        let holders = response["annotations"]
            .get("memory_banks")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let expected = required.map_or_else(|| json!([]), |id| json!([id]));
        println!(
            "{}",
            json!({"diagnostic":"genitive-attribute-control","query":query,
            "selected":selected,"holders":holders,"expected":expected})
        );
        if selected != expected {
            failures.push(format!("{n}: exact selection: {response}"));
        }
        if holders
            .as_array()
            .unwrap()
            .iter()
            .any(|holder| holder != "hook-relevance")
        {
            failures.push(format!("{n}: unexpected holder: {response}"));
        }
        if selected
            .as_array()
            .unwrap()
            .contains(&json!("foreign-pinned"))
        {
            failures.push(format!("{n}: foreign pinned leaked: {response}"));
        }
        if !response["mutations"]["system_prompt"]
            .as_str()
            .unwrap_or("")
            .contains("Preserve the permanent safety rule.")
        {
            failures.push(format!("{n}: permanent rule lost: {response}"));
        }
    }
    assert_eq!(
        executed, 8,
        "all independent genitive controls must execute"
    );
    // A genitive fragment without the certified clause opening remains legacy.
    // This checks evidence preservation, not a precision pass for its selection.
    for query in [
        "Qual cor de embalagem?",
        "Which color of the packaging?",
        "Qual cor de embalagem clara eu uso?",
    ] {
        let (_, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        for id in ["packaging-color", "wall-color"] {
            let e = &evidence[id];
            println!(
                "{}",
                json!({"diagnostic":"genitive-unknown-evidence","query":query,
                "id":id,"attribute_match":e.attribute_match,"attribute_conflict":e.attribute_conflict})
            );
            if !e.attribute_match || e.attribute_conflict {
                failures.push(format!(
                    "unknown genitive must preserve legacy evidence: {query}/{id}: {e:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "genitive precision: {failures:?}");
}

#[tokio::test]
async fn postselect_real_hook_julia_city_does_not_append_a_newer_call() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for fact in corpus["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| {
            ["pt-life-julia-bh", "pt-life-julia-videochamada"]
                .iter()
                .any(|id| fact["key"] == *id)
        })
    {
        let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
        bank.commit(Memory {
            id: fact["key"].as_str().unwrap().into(),
            text: fact["text"].as_str().unwrap().into(),
            tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
            entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let query = "Vou visitar a Júlia em Belo Horizonte, o que eu levo de presente pra ela?";
    let (ranked, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    let reasons = admission_reasons(&ranked, &evidence);
    assert_eq!(
        reasons["pt-life-julia-videochamada"],
        AdmissionReason::NoIndependentRelation
    );
    let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
    let subjects = selected
        .iter()
        .flat_map(|(m, _)| super::subjects(m))
        .collect();
    let notes = bank.tagged(&subjects).await;
    let expanded = with_updates(selected.clone(), &notes, query, &reasons);
    println!(
        "{}",
        json!({"diagnostic":"postselect-stages","query":query,
        "selected":selected.iter().map(|(m,_)|&m.id).collect::<Vec<_>>(),
        "after_updates":expanded.iter().map(|(m,_)|&m.id).collect::<Vec<_>>(),"reasons":reasons})
    );
    assert_eq!(
        expanded
            .iter()
            .map(|(m, _)| m.id.as_str())
            .collect::<Vec<_>>(),
        ["pt-life-julia-bh"]
    );
    let response = hook(
        &deps,
        "postselect-julia",
        json!([{"role":"user","content":query}]),
        context("postselect-julia", "direct", query),
    )
    .await;
    rules_preserved(&response);
    assert_eq!(ids(&response), json!(["pt-life-julia-bh"]));
}

#[tokio::test]
async fn postselect_real_hook_keeps_explicit_city_change_without_unrelated_notes() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, text, at) in [
        ("old-city", "Dana lives in Lisbon.", 1),
        ("new-city", "Dana now lives in Porto.", 2),
        ("laptop-note", "Dana returned her laptop to IT.", 3),
    ] {
        bank.commit(Memory {
            id: id.into(),
            text: text.into(),
            entities: vec!["dana".into()],
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let query = "Where does Dana live?";
    let response = hook(
        &deps,
        "postselect-city",
        json!([{"role":"user","content":query}]),
        context("postselect-city", "direct", query),
    )
    .await;
    rules_preserved(&response);
    assert_eq!(ids(&response), json!(["new-city"]));
}

#[tokio::test]
async fn personal_scope_arguments_and_owners_do_not_borrow_matching_verbs_or_tags() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, text, tags) in [
        (
            "answers",
            "User prefers concise answers. Eu prefiro respostas concisas.",
            vec![],
        ),
        (
            "transport",
            "User prefers binary transport. Eu prefiro transporte binário.",
            vec![],
        ),
        (
            "examples",
            "Examples like concise answers are shown in the handbook.",
            vec![],
        ),
        (
            "liked-notes",
            "User likes ordered notes. Eu gosto de notas ordenadas.",
            vec![],
        ),
        (
            "liked-frames",
            "User likes packet frames. Eu gosto de quadros binários.",
            vec![],
        ),
        (
            "example-like",
            "Formats like ordered notes and packet frames are documented here.",
            vec![],
        ),
        (
            "nickname",
            "Os amigos chamam a usuária de Nara. My friends call me Nara.",
            vec!["nome", "name"],
        ),
        (
            "leisure",
            "Os amigos jogam xadrez. My friends play chess.",
            vec![],
        ),
        ("mine", "User's name is Iris.", vec!["name"]),
        (
            "partner",
            "User's partner's name is Noor.",
            vec!["name", "partner"],
        ),
        ("phone", "Meu fone tem volume baixo quando viajo.", vec![]),
    ] {
        bank.commit(Memory {
            id: id.into(),
            text: text.into(),
            tags: tags.into_iter().map(str::to_owned).collect(),
            entities: Vec::new(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query, expected)) in [
        ("Do I prefer concise answers?", vec!["answers"]),
        ("Do I prefer binary transport?", vec!["transport"]),
        ("Eu prefiro respostas concisas?", vec!["answers"]),
        ("Eu prefiro transporte binário?", vec!["transport"]),
        ("Do I like concise answers?", vec![]),
        ("Do I like ordered notes?", vec!["liked-notes"]),
        ("Do I like packet frames?", vec!["liked-frames"]),
        ("Eu gosto de notas ordenadas?", vec!["liked-notes"]),
        ("Eu gosto de quadros binários?", vec!["liked-frames"]),
        ("como meus amigos me chamar?", vec!["nickname"]),
        ("What do my friends call me?", vec!["nickname"]),
        ("What's my name?", vec!["mine"]),
        ("What's my partner's name?", vec!["partner"]),
        ("Que artistas tocam no meu fone quando viajo?", vec![]),
    ]
    .into_iter()
    .enumerate()
    {
        let session = format!("personal-scope-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        println!(
            "{}",
            json!({"diagnostic":"personal-scope-paired","query":query,"selected":ids(&response),
            "expected":expected,"candidates":hits.iter().map(|(m,_)| {let e=&evidence[&m.id];
                json!({"id":m.id,"spans":e.personal_scope_spans,"terms":e.personal_scope_terms,
                    "scope_matched":e.personal_scope_matched,"personal":e.personal_match,
                    "attribute":e.attribute_match,"predicate_conflict":e.predicate_conflict})}).collect::<Vec<_>>()})
        );
        executed += 1;
        if ids(&response) != json!(expected) {
            failures.push(format!("{n}: {response}"));
        }
        rules_preserved(&response);
    }
    assert_eq!(executed, 14);
    assert!(
        failures.is_empty(),
        "argument/owner precision: {failures:?}"
    );
}

#[tokio::test]
async fn obligation_relation_hook_keeps_neutral_assertions_not_mentions_or_reports() {
    let groups: [(&str, &str, &[&str]); 2] = [
        ("How many seals does a crate need before dispatch?",
            "The depot requires three seals on every crate before dispatch.", &[
                "The manual mentions three seals on every crate as an example.",
                "The depot requires guidance and mentions three seals on every crate before dispatch.",
                "The depot requires three ribbons on every crate before dispatch and mentions three seals.",
                "The depot requires three seals on every pallet before dispatch.",
                "The depot does not require three seals on every crate before dispatch.",
                "If the depot requires three seals on every crate before dispatch, storage changes.",
                "The clerk reports that the depot requires three seals on every crate before dispatch.",
                "The clerk says each crate needs three seals before dispatch.",
                "The manual quotes: each crate needs three seals before dispatch.",
                "\"Each crate needs three seals before dispatch.\"",
                "Each crate needs three seals before dispatch as a fictional example.",
                "Every crate displays three seals before dispatch.",
            ]),
        ("Quantos lacres a caixa precisa antes do envio?",
            "A norma exige três lacres em cada caixa antes do envio.", &[
                "O manual menciona três lacres em cada caixa como exemplo.",
                "A norma exige orientação e menciona três lacres em cada caixa antes do envio.",
                "A norma exige três fitas em cada caixa antes do envio e menciona três lacres.",
                "A norma exige três lacres em cada pacote antes do envio.",
                "A norma não exige três lacres em cada caixa antes do envio.",
                "Se a norma exige três lacres em cada caixa antes do envio, o depósito muda.",
                "O escrivão relata que a norma exige três lacres em cada caixa antes do envio.",
                "O escrivão diz cada caixa precisa de três lacres antes do envio.",
                "O manual cita: cada caixa precisa de três lacres antes do envio.",
                "“Cada caixa precisa de três lacres antes do envio.”",
                "Cada caixa precisa de três lacres antes do envio como exemplo fictício.",
                "Cada caixa exibe três lacres antes do envio.",
            ]),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (language, (query, answer, extras)) in groups.into_iter().enumerate() {
        for (n, extra) in extras.iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            for (id, text) in [("answer", answer), ("incidental", *extra)] {
                bank.commit(Memory {
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
                })
                .await
                .unwrap();
            }
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            // Observe recall evidence before calling the real selection path.
            let (candidates, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            println!(
                "{}",
                json!({"diagnostic":"obligation-relation-before-select", "query":query,
                "candidates":candidates.iter().map(|(m,_)| json!({"id":m.id,"text":m.text,
                    "obligation":evidence[&m.id].target_obligation})).collect::<Vec<_>>()})
            );
            let session = format!("obligation-neutral-{language}-{n}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            executed += 1;
            if ids(&response) != json!(["answer"]) {
                failures.push(format!("{query} / {extra}: {response}"));
            }
            assert_eq!(response["decision"], "continue");
            rules_preserved(&response);
        }
    }
    assert_eq!(executed, 24);
    assert!(failures.is_empty(), "obligation hook: {failures:?}");
}

#[tokio::test]
async fn obligation_relation_hook_preserves_literal_nonobligational_counts() {
    let mut executed = 0;
    for (n, (query, answer)) in [
        (
            "How many tiles does a tray hold?",
            "Each tray holds four tiles.",
        ),
        (
            "Quantas peças o estojo guarda?",
            "Cada estojo guarda quatro peças.",
        ),
        (
            "How many people sit on the payments platform team?",
            "Seven people sit on the payments platform team.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(Memory {
            id: "answer".into(),
            text: answer.into(),
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
        })
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let (_, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert!(evidence["answer"].target_obligation.is_none());
        let session = format!("obligation-literal-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), json!(["answer"]), "{query}: {response}");
        rules_preserved(&response);
        executed += 1;
    }
    assert_eq!(executed, 3);
}

#[tokio::test]
async fn target_relation_review_regressions_hook_keeps_literal_answers_without_incidental_relations(
) {
    let cases = [
        (
            "quantity_wrapper_mention",
            "How many approvals does a pull request need before it can be merged into main?",
            "User's team requires two approvals on every pull request before it can be merged into main.",
            Some("The training guide mentions two approvals on every pull request as a fictional example."),
        ),
        (
            "consumer_disconnected_producer",
            "Which service consumes the payment events Ledgerly publishes to Kafka?",
            "User's Ledgerly service publishes payment events to Kafka topics that the fraud-check service consumes.",
            Some("Ledgerly publishes payment events to Kafka while the archive service consumes the payment events Aster publishes to Orbit."),
        ),
        (
            "quantity_uncertified_prepositional_seam",
            "How many people sit on the payments platform team?",
            "Seven people sit on the payments platform team.",
            None,
        ),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (case, query, answer, incidental)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for (id, text) in [("answer", Some(answer)), ("incidental", incidental)] {
            let Some(text) = text else { continue };
            bank.commit(Memory {
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
            })
            .await
            .unwrap();
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let session = format!("target-review-regression-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let (candidates, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        let expected = json!({
            "memory_ids":["answer"], "decision":"continue", "standing_rule_preserved":true
        });
        let result = json!({
            "memory_ids":ids(&response), "decision":response["decision"],
            "standing_rule_preserved":response["mutations"]["system_prompt"]
                .as_str().is_some_and(|prompt| prompt.contains("Preserve the permanent safety rule."))
        });
        println!(
            "{}",
            json!({
                "diagnostic":"target-relation-review-regressions-hook",
                "run":"memory-quality-523100c4-r1", "phase":"relevance-target-r20-red",
                "attempt":2, "correction_round":20, "case":case,
                "query":query, "text":{"answer":answer,"incidental":incidental},
                "expected":expected, "result":result,
                "candidates":candidates.iter().map(|(m,_)| {
                    let e = &evidence[&m.id];
                    json!({"id":m.id,"text":m.text,
                        "target_scope_required":e.target_scope_required,
                        "target_scope_matched":e.target_scope_matched,
                        "kind":e.target_scope_kind,"terms":e.target_scope_terms,
                        "predicate":e.target_scope_predicate,"subject":e.target_scope_subject,
                        "object":e.target_scope_object,"relative":e.target_scope_relative})
                }).collect::<Vec<_>>()
            })
        );
        executed += 1;
        if result != expected {
            failures.push(format!(
                "{case}: esperado={expected}, obtido={result}; resposta={response}"
            ));
        }
    }
    assert_eq!(
        executed, 3,
        "as três contraprovas devem percorrer o hook real"
    );
    assert!(
        failures.is_empty(),
        "contraprovas de relação-alvo no hook: {failures:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_relation_official_positives_exclude_context_topic_extras_in_all_cache_modes() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
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
            .map(|raw| serde_json::from_str(raw).unwrap())
            .unwrap_or_else(|| json!({"items":{},"model":""}));
        let dir = tempfile::tempdir().unwrap();
        let boundary = super::cached_boundary::boundary(dir.path(), cache.clone()).await;
        let deps = &boundary.deps;
        let mut cfg = (**deps.config.read().await).clone();
        cfg.embeddings_enabled = raw.is_some();
        *deps.config.write().await = Arc::new(cfg);
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for fact in corpus["memories"].as_array().unwrap() {
            let id = fact["key"].as_str().unwrap();
            let text = fact["text"].as_str().unwrap();
            let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
            bank.commit(Memory {
                id: id.into(),
                text: text.into(),
                entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
                tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
                confidence: crate::types::Confidence::Stated,
                corroboration: fact["corroboration"].as_u64().unwrap_or(0) as u32,
                pinned: fact["pinned"].as_bool().unwrap_or(false),
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
            if raw.is_some() {
                assert_eq!(cache["items"][id]["text"], text);
                let vector: Vec<f32> =
                    serde_json::from_value(cache["items"][id]["v"].clone()).unwrap();
                assert_eq!(vector.len(), cache["dims"].as_u64().unwrap() as usize);
                bank.set_vector(id, cache["model"].as_str().unwrap(), text, vector)
                    .await
                    .unwrap();
            }
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let mut expected_calls = Vec::new();
        for (n, (query_id, required)) in [
            ("q-en-work-commit-style", "en-work-conventional-commits"),
            ("q-en-work-pr-approvals", "en-work-review-two-approvals"),
            ("q-en-work-kafka-events", "en-work-ledgerly-kafka"),
        ]
        .into_iter()
        .enumerate()
        {
            let original = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == query_id)
                .unwrap();
            assert_eq!(original["expect"], json!([required]));
            let query = original["text"].as_str().unwrap();
            if raw.is_some() {
                assert_eq!(cache["items"][query_id]["text"], query);
                expected_calls.push(query.to_owned());
            }
            let session = format!("target-official-{lane}-{n}");
            let response = hook(
                deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            println!(
                "{}",
                json!({"diagnostic":"target-relation-official","lane":lane,"query_id":query_id,
                "query":query,"selected":ids(&response),"independent_expected":[required]})
            );
            executed += 1;
            if ids(&response) != json!([required]) {
                failures.push(format!("{lane}/{query_id}: {response}"));
            }
            rules_preserved(&response);
        }
        assert_eq!(
            *boundary.calls.lock().unwrap(),
            expected_calls,
            "each hybrid query must cross the genuine cache boundary, never silently fall back"
        );
    }
    assert_eq!(executed, 9);
    assert!(
        failures.is_empty(),
        "official target relations: {failures:?}"
    );
}

#[tokio::test]
async fn target_relation_inverse_pairs_keep_count_style_consumer_and_database_roles_separate() {
    type PairGroup<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);
    let groups:[PairGroup<'_>;6]=[
        ("style","User writes status notes in the Opaline Ledger style with markers.", &[
            ("Write a status note for this defect using my usual Opaline Ledger markers.","style"),
            ("Which regression test reproduces the defect before a repair?","other")]),
        ("style","A usuária escreve registros claros no padrão Opalino.",&[
            ("Escreva um registro claro para este defeito usando meu padrão Opalino.","style"),
            ("Qual teste de regressão reproduz o defeito antes da correção?","other")]),
        ("count","Each change requires two endorsements before merging.",&[
            ("How many endorsements does a change require before merging?","count"),
            ("How many edited lines does a change contain?","other")]),
        ("count","Cada mudança exige duas confirmações antes da integração.",&[
            ("Quantas confirmações uma mudança exige antes da integração?","count"),
            ("Quantas linhas alteradas uma mudança contém?","other")]),
        ("consumer","The Aster service publishes audit events to Orbit topics that the sentinel service consumes.",&[
            ("Which service consumes the audit events Aster publishes to Orbit?","consumer"),
            ("Which database version does the Aster service use?","other")]),
        ("consumer","O serviço Aster publica eventos de auditoria para tópicos Orbit que o serviço sentinela consome.",&[
            ("Qual serviço consome os eventos de auditoria Aster publica para Orbit?","consumer"),
            ("Qual versão de banco o serviço Aster usa?","other")]),
    ];
    let others = [
        "User writes a regression test that reproduces the defect before any repair.",
        "A usuária escreve um teste de regressão que reproduz o defeito antes da correção.",
        "Each change contains at most 230 edited lines.",
        "Cada mudança contém 230 linhas alteradas.",
        "The Aster service uses database version 8.",
        "O serviço Aster usa banco na versão 8.",
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (g, (id, text, queries)) in groups.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for (id, text) in [(id, text), ("other", others[g])] {
            bank.commit(Memory {
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
            })
            .await
            .unwrap();
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        for (n, (query, expected)) in queries.iter().enumerate() {
            let session = format!("target-pair-{g}-{n}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            let (candidates, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            println!(
                "{}",
                json!({"diagnostic":"target-relation-inverse","query":query,"selected":ids(&response),"expected":[expected],
                "evidence":candidates.iter().map(|(m,_)|{let e=&evidence[&m.id];json!({"id":m.id,
                    "required":e.target_scope_required,"matched":e.target_scope_matched,"kind":e.target_scope_kind,
                    "span":e.target_scope_span,"terms":e.target_scope_terms,"predicate":e.target_scope_predicate,
                    "subject":e.target_scope_subject,"object":e.target_scope_object,"relative":e.target_scope_relative})}).collect::<Vec<_>>()})
            );
            executed += 1;
            if ids(&response) != json!([expected]) {
                failures.push(format!("{g}/{n}: {response}"));
            }
            rules_preserved(&response);
        }
    }
    assert_eq!(executed, 12);
    assert!(
        failures.is_empty(),
        "independent inverse roles: {failures:?}"
    );
}

#[tokio::test]
async fn target_relation_same_words_in_another_clause_do_not_certify_the_requested_role() {
    let cases = [
        (
            "Write a status note for this defect using my usual Opaline Ledger markers.",
            "User writes status notes in the Opaline Ledger style with markers.",
            "User writes status notes and the archivist works in the Opaline Ledger style with markers.",
        ),
        (
            "Escreva um registro claro para este defeito usando meu padrão Opalino.",
            "A usuária escreve registros claros no padrão Opalino.",
            "A usuária escreve registros claros e a arquivista trabalha no padrão Opalino.",
        ),
        (
            "How many endorsements does a change require before merging?",
            "Each change requires two endorsements before merging.",
            "Each change requires guidance and an example mentions two endorsements.",
        ),
        (
            "Quantas confirmações uma mudança exige antes da integração?",
            "Cada mudança exige duas confirmações antes da integração.",
            "Cada mudança exige orientações e um exemplo menciona duas confirmações.",
        ),
        (
            "Which service consumes the audit events Aster publishes to Orbit?",
            "The Aster service publishes audit events to Orbit topics that the sentinel service consumes.",
            "The Aster service publishes audit events to Orbit topics and the archive records a service that the sentinel service consumes.",
        ),
        (
            "Qual serviço consome os eventos de auditoria Aster publica para Orbit?",
            "O serviço Aster publica eventos de auditoria para tópicos Orbit que o serviço sentinela consome.",
            "O serviço Aster publica eventos de auditoria para tópicos Orbit e o arquivo registra algo que o serviço sentinela consome.",
        ),
        (
            "Which database version does the Aster service use?",
            "The Aster service uses database version 8.",
            "The archive uses database version 8 and describes the Aster service.",
        ),
        (
            "Qual versão de banco o serviço Aster usa?",
            "O serviço Aster usa banco na versão 8.",
            "O arquivo usa banco na versão 8 e descreve o serviço Aster.",
        ),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query, answer, incidental)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        for (id, text) in [("answer", answer), ("incidental", incidental)] {
            bank.commit(Memory {
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
            })
            .await
            .unwrap();
        }
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let session = format!("target-clause-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let (candidates, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        println!(
            "{}",
            json!({"diagnostic":"target-relation-clause-binding", "query":query,
            "selected":ids(&response), "independent_expected":["answer"],
            "evidence":candidates.iter().map(|(m,_)| {let e=&evidence[&m.id]; json!({"id":m.id,
                "required":e.target_scope_required,"matched":e.target_scope_matched,"kind":e.target_scope_kind,
                "predicate":e.target_scope_predicate,"subject":e.target_scope_subject,
                "object":e.target_scope_object,"relative":e.target_scope_relative})}).collect::<Vec<_>>() })
        );
        executed += 1;
        if ids(&response) != json!(["answer"]) {
            failures.push(format!("{n}: {response}"));
        }
        rules_preserved(&response);
    }
    assert_eq!(executed, 8);
    assert!(
        failures.is_empty(),
        "target relation clause binding: {failures:?}"
    );
}

#[tokio::test]
async fn subject_inheritance_neutral_hook_retains_explicit_personal_chain_only() {
    let query = "Do I collect ceramic tiles?";
    let cases = [
        ("User nods and collects ceramic tiles.", true),
        (
            "User reports that a cousin nods and collects ceramic tiles.",
            false,
        ),
        (
            "User notices Cousin nods and collects ceramic tiles.",
            false,
        ),
        (
            "User nods and the cousin waves and collects ceramic tiles.",
            false,
        ),
    ];
    let mut failures = Vec::new();
    for (n, (fact, keep)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(Memory {
            id: "candidate".into(),
            text: fact.into(),
            tags: Vec::new(),
            entities: Vec::new(),
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
        })
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let session = format!("subject-neutral-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let expected = if keep {
            json!(["candidate"])
        } else {
            json!([])
        };
        println!(
            "{}",
            json!({"diagnostic":"subject-inheritance-neutral-hook","case":n,
            "query":query,"fact":fact,"expected":expected,"selected":ids(&response)})
        );
        if ids(&response) != expected {
            failures.push(format!("{n}: {response}"));
        }
        if response["decision"] != "continue"
            || !response["mutations"]["system_prompt"]
                .as_str()
                .unwrap_or("")
                .contains("Preserve the permanent safety rule.")
        {
            failures.push(format!("{n}: rule/decision lost: {response}"));
        }
    }
    assert!(failures.is_empty(), "neutral hook chain: {failures:?}");
}

#[tokio::test]
async fn subject_inheritance_real_hook_keeps_personal_positives_and_rejects_foreign_subjects() {
    let cases = [
        (
            "Do I prefer concise answers?",
            "User smiles and prefers concise answers.",
            true,
        ),
        (
            "Do I prefer concise answers?",
            "User says the manager smiles and prefers concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User smiles and the manager prefers concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User says the manager smiles and usually prefers concise answers.",
            false,
        ),
        (
            "Where do I live now?",
            "User moved from Cork to Oslo and now lives in an apartment.",
            true,
        ),
        (
            "Where do I live now?",
            "User says the manager moved from Cork and now lives in Oslo.",
            false,
        ),
        (
            "Where do I live now?",
            "User moved from Cork and the manager now lives in Oslo.",
            false,
        ),
        (
            "Eu estava revisando esquemas?",
            "Eu sorria e estava revisando esquemas.",
            true,
        ),
        (
            "Eu estava revisando esquemas?",
            "Eu digo que o gerente sorri e estava revisando esquemas.",
            false,
        ),
        (
            "Eu estava revisando esquemas?",
            "Eu sorria e o gerente estava revisando esquemas.",
            false,
        ),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query, fact, keep)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(Memory {
            id: "candidate".into(),
            text: fact.into(),
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
        })
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let session = format!("subject-inheritance-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let selected = ids(&response);
        let expected = if keep {
            json!(["candidate"])
        } else {
            json!([])
        };
        let (_, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        println!(
            "{}",
            json!({"diagnostic":"subject-inheritance-real-hook","case":n,
            "query":query,"fact":fact,"expected":expected,"selected":selected,
            "candidate_evidence":evidence.get("candidate").map(|e|json!({
                "matched":e.matched,"scope_spans":e.personal_scope_spans,
                "scope_terms":e.personal_scope_terms,"scope_predicates":e.personal_scope_predicates,
                "scope_required":e.personal_scope_required,"scope_matched":e.personal_scope_matched})),
            "embedding_quality_measured":false})
        );
        executed += 1;
        if selected != expected {
            failures.push(format!("{n}: wrong ids: {response}"));
        }
        if response["decision"] != "continue"
            || !response["mutations"]["system_prompt"]
                .as_str()
                .unwrap_or("")
                .contains("Preserve the permanent safety rule.")
        {
            failures.push(format!("{n}: permanent rule/decision lost: {response}"));
        }
    }
    assert_eq!(executed, 10);
    assert!(
        failures.is_empty(),
        "subject inheritance hook: {failures:?}"
    );
}

#[tokio::test]
async fn certificate_regressions_negated_arguments_do_not_certify_personal_preferences() {
    let cases = [
        (
            "Do I prefer concise answers?",
            "User prefers concise answers.",
            true,
        ),
        (
            "Do I prefer concise answers?",
            "User prefers binary transport, not concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User prefers binary transport and not concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User does not prefer concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User prefers no concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User prefers binary transport and discusses concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers?",
            "User prefers binary transport but another user sends concise answers.",
            false,
        ),
        (
            "Do I prefer concise answers now?",
            "User prefers concise answers.",
            true,
        ),
        (
            "Do I usually prefer concise answers?",
            "User usually prefers concise answers.",
            true,
        ),
        (
            "Eu prefiro respostas concisas?",
            "Eu prefiro respostas concisas.",
            true,
        ),
        (
            "Eu geralmente prefiro respostas concisas?",
            "Eu geralmente prefiro respostas concisas.",
            true,
        ),
        (
            "Eu prefiro respostas concisas?",
            "Eu prefiro transporte binário, não respostas concisas.",
            false,
        ),
        (
            "Eu prefiro respostas concisas?",
            "Eu prefiro transporte binário e não respostas concisas.",
            false,
        ),
        (
            "Eu prefiro respostas concisas?",
            "Eu não prefiro respostas concisas.",
            false,
        ),
        (
            "Eu prefiro respostas concisas?",
            "Eu prefiro transporte binário e menciono respostas concisas.",
            false,
        ),
        (
            "What do my friends send me?",
            "My friends send me postcards.",
            true,
        ),
        (
            "What do my friends send me?",
            "My friends send parcels home.",
            false,
        ),
        (
            "What do my friends send me?",
            "My friends call me Nara.",
            false,
        ),
        ("What's my name?", "User's name is Iris.", true),
        ("What's my name?", "User's name is not Iris.", false),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query, text, keep)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(Memory {
            id: "candidate".into(),
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
        })
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let session = format!("certificate-negative-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        let expected = if keep {
            json!(["candidate"])
        } else {
            json!([])
        };
        let (_, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        println!(
            "{}",
            json!({"diagnostic":"certificate-negation","query":query,"fact":text,
            "selected":ids(&response),"expected":expected,"scopes":evidence.get("candidate").map(|e|
                json!({"terms":e.personal_scope_terms,"predicates":e.personal_scope_predicates,
                    "spans":e.personal_scope_spans,"required":e.personal_scope_required,"matched":e.personal_scope_matched}))})
        );
        executed += 1;
        if ids(&response) != expected {
            failures.push(format!("{n}: {response}"));
        }
        rules_preserved(&response);
    }
    assert_eq!(executed, 20);
    assert!(
        failures.is_empty(),
        "negation/coordination/personal object: {failures:?}"
    );
}

#[tokio::test]
async fn certificate_regressions_procedural_and_open_location_keep_independently_supported_facts() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for fact in corpus["memories"].as_array().unwrap().iter().filter(|f| {
        ["en-work-flyway-migrations", "upd-city-old", "upd-city-new"]
            .iter()
            .any(|key| f["key"] == *key)
    }) {
        let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
        bank.commit(Memory {
            id: fact["key"].as_str().unwrap().into(),
            text: fact["text"].as_str().unwrap().into(),
            entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
            tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query, required)) in [
        (
            "How do I apply database schema migrations?",
            "en-work-flyway-migrations",
        ),
        (
            "I need to add a column; how do I usually write Flyway schema migrations?",
            "en-work-flyway-migrations",
        ),
        (
            "How do I currently write Flyway schema migrations?",
            "en-work-flyway-migrations",
        ),
        (
            "Where do I live now? Looking for a good bakery near my place.",
            "upd-city-new",
        ),
        ("Where do I currently live?", "upd-city-new"),
    ]
    .into_iter()
    .enumerate()
    {
        let session = format!("certificate-positive-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        println!(
            "{}",
            json!({"diagnostic":"certificate-supported-positive","query":query,
            "selected":ids(&response),"required":required})
        );
        executed += 1;
        if !ids(&response)
            .as_array()
            .unwrap()
            .contains(&json!(required))
            || ids(&response)
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id != required && id != "upd-city-old")
        {
            failures.push(format!("{n}: {response}"));
        }
        rules_preserved(&response);
    }
    assert_eq!(executed, 5);
    assert!(
        failures.is_empty(),
        "supported procedural/location: {failures:?}"
    );
}

#[tokio::test]
async fn personal_scope_official_groups_keep_factual_loss_distinct_from_safe_abstention() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for fact in corpus["memories"].as_array().unwrap() {
        let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
        bank.commit(Memory {
            id: fact["key"].as_str().unwrap().into(),
            text: fact["text"].as_str().unwrap().into(),
            tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
            entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: fact["pinned"].as_bool().unwrap_or(false),
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
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, (query_id, expected)) in [
        ("q-pt-life-par-artistas", vec!["pt-life-mpb-gal"]),
        ("q-en-work-wire-protocol", vec!["en-work-grpc-internal"]),
        ("q-id-partner", vec!["id-partner"]),
        ("q-id-apelido", vec!["id-apelido"]),
        ("q-id-name", vec!["id-full-name"]),
    ]
    .into_iter()
    .enumerate()
    {
        let original = corpus["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == query_id)
            .unwrap();
        let query = original["text"].as_str().unwrap();
        let session = format!("personal-official-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        println!(
            "{}",
            json!({"diagnostic":"personal-scope-official","query_id":query_id,
            "query":query,"selected":ids(&response),"original_factual_expected":original["expect"],
            "independent_policy_expected":expected,
            "disposition":if expected.is_empty(){"no demonstrable owner/argument certificate; factual missing is not recall"}
                else{"explicit naming/owned attribute must remain"}})
        );
        executed += 1;
        if ids(&response) != json!(expected) {
            failures.push(format!("{query_id}: {response}"));
        }
        rules_preserved(&response);
    }
    let query="Remind me, do I prefer long detailed explanations or straight-to-the-point answers from you?";
    let response = hook(
        &deps,
        "personal-official-style",
        json!([{"role":"user","content":query}]),
        context("personal-official-style", "direct", query),
    )
    .await;
    executed += 1;
    println!(
        "{}",
        json!({"diagnostic":"personal-scope-official","query":query,"selected":ids(&response),
        "original_factual_expected":["evt-pref-concise"],"independent_policy_expected":["evt-pref-concise"],
        "disposition":"owned assistant-output style relation proves concise answers; transport preference remains unrelated"})
    );
    if ids(&response) != json!(["evt-pref-concise"]) {
        failures.push(format!("answer-style: {response}"));
    }
    rules_preserved(&response);
    assert_eq!(executed, 6);
    assert!(
        failures.is_empty(),
        "official personal groups: {failures:?}"
    );
}

#[tokio::test]
async fn wh_preservation_subject_question_keeps_current_ci_and_unknown_verbal_sequences() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for fact in corpus["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| {
            ["upd-ci-old", "upd-ci-new"]
                .iter()
                .any(|key| fact["key"] == *key)
        })
    {
        let at = now_ms() - fact["age_days"].as_u64().unwrap() * 86_400_000;
        bank.commit(Memory {
            id: fact["key"].as_str().unwrap().into(),
            text: fact["text"].as_str().unwrap().into(),
            tags: serde_json::from_value(fact["tags"].clone()).unwrap(),
            entities: serde_json::from_value(fact["entities"].clone()).unwrap(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for (n, text) in [
        "Recall which CI replaced Jenkins at work.",
        "Which CI replaced Jenkins at work?",
        "Qual CI substituiu Jenkins no trabalho?",
        "Which CI replaced Jenkins pipeline at work?",
    ]
    .into_iter()
    .enumerate()
    {
        let session = format!("wh-preservation-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        executed += 1;
        let selected = ids(&response);
        let (_, evidence) = bank
            .recall_evidence(text, None, "", usize::MAX, 30, false, 0.0)
            .await;
        println!(
            "{}",
            json!({"diagnostic":"wh-preservation","query":text,"selected":selected,
            "attribute_conflicts":evidence.iter().map(|(id,e)|json!({"id":id,"attribute_conflict":e.attribute_conflict,
                "matched":e.matched,"relation_terms":e.relation_terms})).collect::<Vec<_>>()})
        );
        if !selected.as_array().unwrap().contains(&json!("upd-ci-new"))
            || !selected
                .as_array()
                .unwrap()
                .iter()
                .all(|id| [json!("upd-ci-old"), json!("upd-ci-new")].contains(id))
            || evidence
                .get("upd-ci-new")
                .is_none_or(|e| e.attribute_conflict)
        {
            failures.push(format!("{text}: {response}"));
        }
        rules_preserved(&response);
    }
    assert_eq!(executed, 4);
    assert!(
        failures.is_empty(),
        "WH verbal/unknown must preserve current recall: {failures:?}"
    );
    // Independent names/predicate: the production guard must not recognize CI,
    // Jenkins or the particular verb from the original regression.
    relational_sufficiency_case(
        "Which storage displaced Pebble at work?",
        &[(
            "answer",
            "Migrei storage da aplicação do Pebble para Slate.",
        )],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn fallback_policy_real_hook_abstains_without_support_and_preserves_lexical_positives() {
    for (language, fact, supported_query, unsupported_query) in [
        (
            "pt",
            "Eu corro no horário da manhã.",
            "Em que horário eu corro?",
            "Em que período eu faço jogging?",
        ),
        (
            "en",
            "We run at the same time each morning.",
            "At what time do we run?",
            "During what interval do we jog?",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(Memory {
            id: "answer".into(),
            text: fact.into(),
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
        })
        .await
        .unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.")
            .unwrap();
        let labels = super::evaluator::Labels {
            required: vec!["answer".into()],
            ..super::evaluator::Labels::default()
        };
        for (n, (query, requires_recall)) in [(supported_query, true), (unsupported_query, false)]
            .into_iter()
            .enumerate()
        {
            let session = format!("fallback-policy-{language}-{n}");
            let response = hook(
                &deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            let selected: Vec<String> = serde_json::from_value(ids(&response)).unwrap();
            let (_, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let supported = evidence
                .iter()
                .filter(|(_, e)| super::evaluator::lexical_support(e))
                .map(|(id, _)| id.clone())
                .collect();
            let policy = labels.injection_policy(&selected, &supported, false);
            println!(
                "{}",
                json!({"diagnostic":"fallback-policy-real-hook","language":language,
                "query":query,"selected":selected,"policy":policy,"embedding_quality_measured":false})
            );
            rules_preserved(&response);
            assert_eq!(
                selected,
                if requires_recall {
                    vec!["answer".to_owned()]
                } else {
                    vec![]
                }
            );
            assert!(policy.integral_ok);
            assert_eq!(policy.abstention_without_support, !requires_recall);
            assert_eq!(
                policy.factual.required_recall,
                Some(if requires_recall { 1.0 } else { 0.0 })
            );
        }
    }
}

// All facts are lexical, untagged and entity-free. Each case uses a fresh bank;
// the foreign pinned twin must not supply evidence or appear in the response.
async fn relational_sufficiency_case(query: &str, facts: &[(&str, &str)], expected: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    let (foreign, _) = store.ensure_bank("unrelated-bank", None).await.unwrap();
    for (id, text, pinned, target) in facts
        .iter()
        .map(|(id, text)| (*id, *text, false, &bank))
        .chain(std::iter::once(("foreign-pinned", query, true, &foreign)))
    {
        target
            .commit(Memory {
                id: id.into(),
                text: text.into(),
                entities: Vec::new(),
                tags: Vec::new(),
                confidence: crate::types::Confidence::Stated,
                corroboration: 0,
                pinned,
                source: None,
                created_at: now_ms(),
                updated_at: now_ms(),
                stated_at: None,
                invalid_at: None,
                superseded_by: None,
                revision: 0,
            })
            .await
            .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let response = hook(
        &deps,
        "relational-case",
        json!([{"role":"user","content":query}]),
        context("relational-case", "direct", query),
    )
    .await;
    let (hits, evidence) = bank
        .recall_evidence(query, None, "", 10, 30, false, 0.0)
        .await;
    let candidates: Vec<Value> = hits
        .iter()
        .map(|(m, score)| {
            let e = &evidence[&m.id];
            json!({"id":m.id,"score":score,"matched":e.matched,"terms":e.terms,
            "relation_matched":e.relation_matched,"requested_slot_matched":e.requested_slot_matched,
            "requested_slot_spans":e.requested_slot_spans.as_ref()
                .map(|spans|spans.iter().map(|s|[s.start,s.end]).collect::<Vec<_>>()),
            "projection_unknown":e.requested_slot_spans.is_none(),
            "attribute_match":e.attribute_match,"personal_match":e.personal_match,
            "cosine":e.cosine,"vector":e.vector})
        })
        .collect();
    println!(
        "{}",
        json!({"diagnostic":"relational-sufficiency","query":query,
        "selected":ids(&response),"expected":expected,"candidates":candidates,
        "embedding_quality_measured":false})
    );
    rules_preserved(&response);
    assert!(
        !ids(&response)
            .as_array()
            .unwrap()
            .contains(&json!("foreign-pinned")),
        "a pinned fact in another bank cannot anchor or leak: {response}"
    );
    assert_eq!(
        ids(&response),
        json!(expected),
        "relational sufficiency: {query}: {response}"
    );
}

#[tokio::test]
async fn relational_sufficiency_pt_singleton_answer_survives_requested_slot() {
    relational_sufficiency_case(
        "Em que horário eu corro?",
        &[("answer", "Eu corro no horário da manhã.")],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_en_singleton_answer_survives_requested_slot() {
    relational_sufficiency_case(
        "At what time do we run?",
        &[("answer", "We run at the same time each morning.")],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_pt_answer_is_not_lost_or_joined_by_slot_only() {
    relational_sufficiency_case(
        "Em que horário eu corro?",
        &[
            ("answer", "Eu corro no horário da manhã."),
            ("slot-only", "O horário do ônibus está no painel."),
        ],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_en_answer_is_not_lost_or_joined_by_slot_only() {
    relational_sufficiency_case(
        "At what time do we run?",
        &[
            ("answer", "We run at the same time each morning."),
            ("slot-only", "The time of the next ferry is on the board."),
        ],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_two_relation_terms_keep_answer_and_exclude_slot_only() {
    relational_sufficiency_case(
        "Em que horário eu reviso o desenho?",
        &[
            ("answer", "Eu reviso o desenho no horário reservado."),
            ("slot-only", "O horário do ônibus está no painel."),
        ],
        &["answer"],
    )
    .await;
    relational_sufficiency_case(
        "At what time do we inspect the draft?",
        &[
            ("answer", "We inspect the draft at the agreed time."),
            ("slot-only", "The time of the next ferry is on the board."),
        ],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_slot_only_never_anchors_even_without_a_competing_answer() {
    relational_sufficiency_case(
        "Em que horário eu corro?",
        &[("slot-only", "O horário do ônibus está no painel.")],
        &[],
    )
    .await;
    relational_sufficiency_case(
        "At what time do we run?",
        &[("slot-only", "The time of the next ferry is on the board.")],
        &[],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_one_incidental_match_is_not_a_complete_relation() {
    relational_sufficiency_case(
        "Em que horário eu corro no parque?",
        &[("incidental", "O horário do parque está no painel.")],
        &[],
    )
    .await;
    relational_sufficiency_case(
        "At what time do we run in the park?",
        &[("incidental", "The time for the park is on the board.")],
        &[],
    )
    .await;
}

#[tokio::test]
async fn relational_sufficiency_unknown_fragments_and_quotes_preserve_prior_behavior() {
    relational_sufficiency_case(
        "Em que horário?",
        &[("subject", "O horário do ônibus está no painel.")],
        &["subject"],
    )
    .await;
    relational_sufficiency_case(
        "At what time?",
        &[("subject", "The time of the next ferry is on the board.")],
        &["subject"],
    )
    .await;
    relational_sufficiency_case(
        "Em que horário \"eu\" corro?",
        &[("answer", "Eu corro no horário da manhã.")],
        &["answer"],
    )
    .await;
    relational_sufficiency_case(
        "At what time do \"we\" run?",
        &[("answer", "We run at the same time each morning.")],
        &["answer"],
    )
    .await;
}

#[tokio::test]
async fn requested_attributes_and_portuguese_inflections_preserve_existing_positives() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for fact in corpus["memories"].as_array().unwrap() {
        bank.commit(Memory {
            id: fact["key"].as_str().unwrap().into(),
            text: fact["text"].as_str().unwrap().into(),
            entities: serde_json::from_value(fact["entities"].clone()).unwrap_or_default(),
            tags: serde_json::from_value(fact["tags"].clone()).unwrap_or_default(),
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: fact["pinned"].as_bool().unwrap_or(false),
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            stated_at: None,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        })
        .await
        .unwrap();
    }
    for (n, (id, required)) in [
        ("q-id-apelido", "id-apelido"),
        ("q-upd-ci-pipeline", "upd-ci-new"),
        ("q-evt-units", "evt-pref-metric"),
        ("q-evt-idioma", "evt-pref-idioma"),
    ]
    .iter()
    .enumerate()
    {
        let text = corpus["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == *id)
            .unwrap()["text"]
            .as_str()
            .unwrap();
        let session = format!("attribute-positive-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        assert!(
            ids(&response)
                .as_array()
                .unwrap()
                .contains(&json!(required)),
            "{id}: {response}"
        );
        let selected = ids(&response);
        let admitted = selected.as_array().unwrap();
        if *id == "q-upd-ci-pipeline" {
            assert!(
                admitted.iter().all(|id| ["upd-ci-new", "upd-ci-old"]
                    .iter()
                    .any(|allowed| id == allowed)),
                "CI attribute must not inject family calls: {response}"
            );
        }
    }
    for (n, text) in [
        "Me ajude a começar um projeto novo sem decisões anteriores.",
        "Qual é o formato de um arquivo PNG?",
        "Você sabe a capital de Portugal?",
        "What Postgres version does Rotafy use?",
    ]
    .iter()
    .enumerate()
    {
        let session = format!("attribute-negative-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        assert_eq!(ids(&response), json!([]), "{text}: {response}");
    }
}

#[tokio::test]
async fn selector_final_preserves_personal_questions_without_unrelated_predicate_veto() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for f in corpus["memories"].as_array().unwrap() {
        let at = now_ms() - f["age_days"].as_u64().unwrap() * 86_400_000;
        bank.commit(Memory {
            id: f["key"].as_str().unwrap().into(),
            text: f["text"].as_str().unwrap().into(),
            entities: serde_json::from_value(f["entities"].clone()).unwrap_or_default(),
            tags: serde_json::from_value(f["tags"].clone()).unwrap_or_default(),
            pinned: f["pinned"].as_bool().unwrap_or(false),
            confidence: crate::types::Confidence::Stated,
            corroboration: f["corroboration"].as_u64().unwrap_or(0) as u32,
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
    }
    for (n, (id, required)) in [
        ("q-id-namorado", "id-partner"),
        ("q-upd-diet-recipe", "upd-diet-new"),
        ("q-upd-manager", "upd-manager-new"),
    ]
    .iter()
    .enumerate()
    {
        let text = corpus["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == *id)
            .unwrap()["text"]
            .as_str()
            .unwrap();
        let session = format!("selector-final-{n}");
        let out = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        let selected = ids(&out);
        let alternatives = if *required == "id-partner" {
            vec!["id-partner", "dup-partner"]
        } else {
            vec![*required]
        };
        assert!(
            selected
                .as_array()
                .unwrap()
                .iter()
                .any(|id| alternatives.iter().any(|a| id == a)),
            "{id}: {out}"
        );
        if *id == "q-upd-manager" {
            assert!(
                selected
                    .as_array()
                    .unwrap()
                    .contains(&json!("upd-manager-new")),
                "new manager cannot be dropped while old remains: {out}"
            );
        }
    }
}

#[tokio::test]
async fn selector_final_named_subject_is_not_an_incidental_object() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.commit(Memory {
        id: "regional-timezone".into(),
        text: "The user works in Portugal time.".into(),
        entities: vec!["portugal".into()],
        tags: Vec::new(),
        pinned: false,
        confidence: crate::types::Confidence::Stated,
        corroboration: 0,
        source: None,
        created_at: now_ms(),
        updated_at: now_ms(),
        stated_at: None,
        invalid_at: None,
        superseded_by: None,
        revision: 0,
    })
    .await
    .unwrap();
    let text = "What is the capital of Portugal?";
    let out = hook(
        &deps,
        "incidental-country",
        json!([{"role":"user","content":text}]),
        context("incidental-country", "direct", text),
    )
    .await;
    assert_eq!(
        ids(&out),
        json!([]),
        "country as object of an unknown attribute is not a direct subject question: {out}"
    );
}

#[tokio::test]
async fn relevance_r5_compound_attribute_and_receipt_intent_keep_direct_authority() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    for f in corpus["memories"].as_array().unwrap().iter().filter(|f| {
        [
            "upd-editor-old",
            "upd-editor-new",
            "en-work-testcontainers",
            "en-work-flyway-migrations",
            "en-work-go-tools",
            "upd-tz-new",
        ]
        .iter()
        .any(|id| f["key"] == *id)
    }) {
        let at = now_ms() - f["age_days"].as_u64().unwrap() * 86_400_000;
        bank.commit(Memory {
            id: f["key"].as_str().unwrap().into(),
            text: f["text"].as_str().unwrap().into(),
            entities: serde_json::from_value(f["entities"].clone()).unwrap(),
            tags: serde_json::from_value(f["tags"].clone()).unwrap(),
            pinned: false,
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
    }
    let editor = corpus["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["id"] == "q-upd-editor")
        .unwrap()["text"]
        .as_str()
        .unwrap();
    for (n, text) in [
        editor,
        "Which code editor do I use?",
        "What code editor is configured for me?",
    ]
    .iter()
    .enumerate()
    {
        let session = format!("r5-editor-{n}");
        let input:PreGenerateInput=serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"step":0,"recall_context":context(&session,"direct",text)})).unwrap();
        assert_eq!(recall_query(&input).as_deref(), Some(*text));
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        let selected = ids(&response);
        assert!(
            selected
                .as_array()
                .unwrap()
                .contains(&json!("upd-editor-new")),
            "{text}: {response}"
        );
        assert!(
            selected
                .as_array()
                .unwrap()
                .iter()
                .all(|id| ["upd-editor-new", "upd-editor-old"]
                    .iter()
                    .any(|allowed| id == allowed)),
            "editor question must not borrow the action using from database testing: {response}"
        );
        rules_preserved(&response);
    }
    for (n,text) in [
        "Technical notification: the database migration worker finished with exit 0. Acknowledge receipt only.",
        "The database schema task finished. Confirm receipt only.",
        "O processamento do banco terminou. Apenas registre o recebimento.",
        "Acknowledge receipt only.",
        "Apenas confirme o recebimento.",
    ].iter().enumerate() {
        let session=format!("r5-receipt-{n}");
        let input:PreGenerateInput=serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"step":0,"recall_context":context(&session,"direct",text)})).unwrap();
        assert_eq!(recall_query(&input).as_deref(),Some(*text),"intent must not rewrite the controlled direct query or its origin");
        let response=hook(&deps,&session,json!([{"role":"user","content":text,"origin":{"kind":"direct"}}]),context(&session,"direct",text)).await;
        assert_eq!(ids(&response),json!([]),"receipt scope needs no stored personal facts: {response}");
        rules_preserved(&response);
    }
    for (n, text) in [
        "How do I apply database schema migrations?",
        "Technical notification: how are database schema migrations applied?",
        "Acknowledge receipt and explain the database migration process.",
        "Explain the database migration process. Acknowledge receipt only.",
        "Describe the database migration process. Acknowledge receipt only.",
        "Describe migrations: the database task finished. Acknowledge receipt only.",
        "Summarize the database migration process. Acknowledge receipt only.",
        "Compare database migration approaches. Confirm receipt only.",
        "The database schema process is ready. Acknowledge receipt only.",
        "The database migration completed with instructions to describe the process. Acknowledge receipt only.",
    ]
    .iter()
    .enumerate()
    {
        let session = format!("r5-migration-{n}");
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text}]),
            context(&session, "direct", text),
        )
        .await;
        assert!(ids(&response).as_array().unwrap().contains(&json!("en-work-flyway-migrations")),"a real factual request must still recall, regardless of notification-like text: {response}");
        rules_preserved(&response);
    }
}

fn offline(dir: &std::path::Path) -> Arc<Deps> {
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
    Arc::new(Deps {
        emitter: Arc::new(crate::events::Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(
            crate::store::Store::open(dir.to_path_buf()).unwrap(),
        ))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
            embeddings_enabled: false,
            extraction_enabled: false,
            ..WorkerConfig::default()
        }))),
    })
}

async fn seed(deps: &Deps) {
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, text) in [
        ("ledger", "Ledgerly uses PostgreSQL 16."),
        ("coffee", "O café de torra clara é preparado no V60."),
        ("go", "The gateway is written in Go."),
    ] {
        bank.commit(Memory {
            id: id.into(),
            text: text.into(),
            entities: Vec::new(),
            tags: Vec::new(),
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: id == "ledger",
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            stated_at: None,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
}

fn context(session: &str, kind: &str, text: &str) -> Value {
    json!({
        "version": 1,
        "session_id": session,
        "turn_id": format!("turn-{session}"),
        "source": {"kind": kind, "entry_id": "e-current"},
        "query": {"entry_id": "e-current", "text": text}
    })
}

async fn hook(deps: &Arc<Deps>, session: &str, messages: Value, context: Value) -> Value {
    let input: PreGenerateInput = serde_json::from_value(json!({
        "session_id": session,
        "turn_id": format!("turn-{session}"),
        "step": 0,
        "metadata": {"memory_bank": "hook-relevance"},
        "recall_context": context,
        "generate": {"system_prompt": "Offline regression", "messages": messages}
    }))
    .unwrap();
    serde_json::to_value(pre_generate(deps, input).await.unwrap()).unwrap()
}

fn ids(response: &Value) -> Value {
    response["annotations"]
        .get("memory_ids")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn rules_preserved(response: &Value) {
    assert_eq!(response["decision"], "continue");
    assert!(response["mutations"]["system_prompt"]
        .as_str()
        .unwrap()
        .contains("Preserve the permanent safety rule."));
}

#[tokio::test]
async fn relevance_contract_current_human_query_ignores_later_model_notices() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let response = hook(
        &deps,
        "current-human",
        json!([
            {"role":"user", "content":"O café de torra clara é preparado no V60."},
            {"role":"user", "content":"Which PostgreSQL does Ledgerly use?"},
            {"role":"user", "content":"Skills updated: Go gateway notification."}
        ]),
        context(
            "current-human",
            "direct",
            "Which PostgreSQL does Ledgerly use?",
        ),
    )
    .await;
    assert_eq!(ids(&response), json!(["ledger"]), "{response}");
    rules_preserved(&response);
}

#[tokio::test]
async fn relevance_contract_notification_abstains_without_dropping_rules_or_pin() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let response = hook(
        &deps,
        "notification",
        json!([{"role":"user", "content":"Ledgerly PostgreSQL 16 completed"}]),
        context(
            "notification",
            "notification",
            "Ledgerly PostgreSQL 16 completed",
        ),
    )
    .await;
    assert_eq!(ids(&response), json!([]), "{response}");
    rules_preserved(&response);
    assert!(
        deps.store()
            .await
            .bank("hook-relevance")
            .await
            .unwrap()
            .get("ledger")
            .await
            .unwrap()
            .pinned
    );
}

#[tokio::test]
async fn relevance_contract_legacy_unknown_and_mismatched_context_are_rules_only() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    for (session, mut source) in [
        ("legacy", Value::Null),
        (
            "unknown",
            context("unknown", "future-kind", "Ledgerly PostgreSQL"),
        ),
        (
            "mismatch",
            context("other-session", "direct", "Ledgerly PostgreSQL"),
        ),
        (
            "version",
            context("version", "direct", "Ledgerly PostgreSQL"),
        ),
    ] {
        if session == "version" {
            source["version"] = json!(99);
        }
        let response = hook(
            &deps,
            session,
            json!([{"role":"user", "content":"Ledgerly PostgreSQL", "origin":{"kind":"direct"}}]),
            source,
        )
        .await;
        assert_eq!(ids(&response), json!([]), "{session}: {response}");
        rules_preserved(&response);
    }
}

#[tokio::test]
async fn relevance_contract_continuation_needs_explicit_bound_prior_query_pt_en() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    for (session, text, prior, expected) in [
        (
            "continuation-en",
            "Continue with the details.",
            "Which PostgreSQL does Ledgerly use?",
            "ledger",
        ),
        (
            "continuation-pt",
            "Pode continuar com os detalhes.",
            "Explique o café de torra clara no V60.",
            "coffee",
        ),
    ] {
        let mut source = context(session, "direct", text);
        source["continuation"] = json!({"entry_id":"e-prior", "text":prior});
        let response = hook(
            &deps,
            session,
            json!([
                {"role":"user", "content":prior},
                {"role":"assistant", "content":"A prior response."},
                {"role":"user", "content":text}
            ]),
            source,
        )
        .await;
        assert_eq!(ids(&response), json!([expected]), "{session}: {response}");
        rules_preserved(&response);
    }
}

#[tokio::test]
async fn relevance_contract_new_request_does_not_inherit_prior_topic() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let text = "Design a new observatory project; go through its initial requirements.";
    let response = hook(
        &deps,
        "new-request",
        json!([
            {"role":"user", "content":"Which PostgreSQL does Ledgerly use?"},
            {"role":"user", "content":text}
        ]),
        context("new-request", "direct", text),
    )
    .await;
    assert_eq!(
        ids(&response),
        json!([]),
        "an incidental Go match must not anchor: {response}"
    );
    rules_preserved(&response);
}

#[tokio::test]
async fn relevance_contract_incidental_words_do_not_anchor_across_small_bank_hits() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, text) in [
        ("great", "The great deployment uses Bazel."),
        ("carry", "Workers carry release artifacts."),
    ] {
        bank.commit(Memory {
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
        })
        .await
        .unwrap();
    }
    let response = hook(
        &deps,
        "split-incidental",
        json!([{"role":"user", "content":"Great, carry on."}]),
        context("split-incidental", "direct", "Great, carry on."),
    )
    .await;
    assert_eq!(ids(&response), json!([]), "{response}");
}

#[tokio::test]
async fn relevance_contract_literal_notification_text_does_not_authenticate_origin() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let response = hook(
        &deps,
        "human-quoted-notice",
        json!([{"role":"user", "content":"[notification] Which PostgreSQL does Ledgerly use?", "origin":{"kind":"notification"}}]),
        context("human-quoted-notice", "direct", "[notification] Which PostgreSQL does Ledgerly use?"),
    ).await;
    assert_eq!(
        ids(&response),
        json!(["ledger"]),
        "text and message claims must not suppress a human query: {response}"
    );
    rules_preserved(&response);
}

#[tokio::test]
async fn relevance_contract_visible_memory_is_not_appended_again() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let first = hook(
        &deps,
        "visible-first",
        json!([{"role":"user", "content":"Which PostgreSQL does Ledgerly use?"}]),
        context(
            "visible-first",
            "direct",
            "Which PostgreSQL does Ledgerly use?",
        ),
    )
    .await;
    assert_eq!(ids(&first), json!(["ledger"]));
    let mut messages =
        vec![json!({"role":"user", "content":"Which PostgreSQL does Ledgerly use?"})];
    messages.extend(
        first["mutations"]["append_messages"]
            .as_array()
            .unwrap()
            .iter()
            .cloned(),
    );
    let second = hook(
        &deps,
        "visible-second",
        json!(messages),
        context(
            "visible-second",
            "direct",
            "Which PostgreSQL does Ledgerly use?",
        ),
    )
    .await;
    assert_eq!(ids(&second), json!([]), "{second}");
    rules_preserved(&second);
}

#[tokio::test]
async fn relevance_diagnostic_incidental_best_bm25_is_normalized_to_one() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let store = deps.store().await;
    let bank = store.bank("hook-relevance").await.unwrap();
    let query = "Design a new observatory project; go through its initial requirements.";
    let maximum = bank.bm25_max(query, |_| true).await;
    let (ranked, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, maximum)
        .await;
    assert!(maximum > 0.0);
    let go = evidence.get("go").unwrap();
    assert_eq!(go.matched, ["go"]);
    assert!(go.terms > 2);
    assert!((go.lexical - 1.0).abs() < 0.0001);
    assert!(!go.vector && go.cosine.is_none() && go.z.is_none());
    assert!(ranked.iter().any(|(m, _)| m.id == "go"));
    println!(
        "{}",
        json!({
            "diagnostic":"incidental-query-best",
            "query":query,
            "bm25_max":maximum,
            "matched":go.matched,
            "terms":go.terms,
            "lexical":go.lexical,
            "relevance":go.relevance,
            "semantic":"disabled; not measured",
            "layers":[],
            "selected":select(ranked, &evidence, &HashSet::new(), 6, query)
                .into_iter().map(|(m, _)|m.id).collect::<Vec<_>>()
        })
    );
}

#[tokio::test]
async fn relevance_contract_invoked_agent_allowed_but_malformed_context_never_promoted() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let text = "Which PostgreSQL does Ledgerly use?";
    let allowed = hook(
        &deps,
        "agent-allowed",
        json!([{"role":"user","content":text}]),
        context("agent-allowed", "invoked-agent", text),
    )
    .await;
    assert_eq!(ids(&allowed), json!(["ledger"]));
    for n in 0..5 {
        let session = format!("bad-link-{n}");
        let mut source = context(&session, "direct", text);
        match n {
            0 => source["turn_id"] = json!("other-turn"),
            1 => source["query"]["entry_id"] = json!("other-entry"),
            2 => source["continuation"] = json!({"entry_id":"e-current","text":text}),
            3 => source["query"]["text"] = json!("x".repeat(3201)),
            _ => source = json!("malformed"),
        }
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":text,"origin":{"kind":"direct"}}]),
            source,
        )
        .await;
        assert_eq!(ids(&response), json!([]), "{response}");
        rules_preserved(&response);
    }
}

#[tokio::test]
async fn relevance_contract_later_step_and_foreign_bank_are_not_recalled() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    seed(&deps).await;
    let mut raw = json!({"session_id":"later","turn_id":"t-later","step":1,
        "metadata":{"memory_bank":"hook-relevance"},
        "recall_context":{"version":1,"session_id":"later","turn_id":"t-later","source":{"kind":"direct","entry_id":"e1"},"query":{"entry_id":"e1","text":"Ledgerly PostgreSQL"}},
        "generate":{"system_prompt":"offline","messages":[]}});
    let out = serde_json::to_value(
        pre_generate(&deps, serde_json::from_value(raw.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ids(&out), json!([]));
    rules_preserved(&out);
    raw["step"] = json!(0);
    raw["session_id"] = json!("empty-bank");
    raw["metadata"]["memory_bank"] = json!("empty-bank");
    raw["recall_context"]["session_id"] = json!("empty-bank");
    let out = serde_json::to_value(
        pre_generate(&deps, serde_json::from_value(raw).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ids(&out),
        json!([]),
        "must never fall back to hook-relevance: {out}"
    );
}

// Review reproductions: expectations come from the user contract (explicit
// user->assistant directive needs auditable target/executor/domain/scope before
// select; absent proof abstains; supported positives stay), not from helpers.
async fn directive_review_run(
    label: &str,
    query: &str,
    pool: &[(&str, &str)],
    expected: &[&str],
) -> Vec<String> {
    let mut failures = Vec::new();
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    for (id, text) in pool {
        bank.commit(Memory {
            id: (*id).into(),
            text: (*text).into(),
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
        })
        .await
        .unwrap();
    }
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let (ranked, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    let reasons = admission_reasons(&ranked, &evidence);
    let examined: Vec<&str> = pool.iter().map(|(id, _)| *id).collect();
    println!(
        "{}",
        json!({
            "diagnostic":"directive-relation-review","stage":"before_select",
            "case":label,"query":query,"independent_expected":expected,
            "candidate_evidence":directive_candidate_audit(&ranked,&evidence,&examined)
        })
    );
    for id in &examined {
        let supported = reasons.get(*id).is_some_and(|reason| reason.supported());
        let want = expected.contains(id);
        if supported != want {
            failures.push(format!("{label}/{id}: certificado {supported} != {want}"));
        }
    }
    let session = format!("directive-review-{label}");
    let response = hook(
        &deps,
        &session,
        json!([{"role":"user","content":query}]),
        context(&session, "direct", query),
    )
    .await;
    println!(
        "{}",
        json!({
            "diagnostic":"directive-relation-review","stage":"hook_result",
            "case":label,"independent_expected":expected,"selected":ids(&response)
        })
    );
    if ids(&response) != json!(expected) {
        failures.push(format!("{label}: seleção: {response}"));
    }
    rules_preserved(&response);
    failures
}

#[tokio::test]
async fn directive_relation_review_colon_framed_code_request_keeps_code_and_excludes_prose() {
    let query =
        "Vou te pedir um código: os nomes de variáveis e os comentários devem ficar em que idioma?";
    let code = "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.";
    let prose = "A usuária pede que a prosa escrita pelo assistente, incluindo nomes de variáveis e comentários, fique em catalão.";
    let mut failures = Vec::new();
    failures.extend(directive_review_run("f2-prose-alone", query, &[("prose", prose)], &[]).await);
    failures
        .extend(directive_review_run("f2-code-alone", query, &[("code", code)], &["code"]).await);
    failures.extend(
        directive_review_run(
            "f2-pool",
            query,
            &[("code", code), ("prose", prose)],
            &["code"],
        )
        .await,
    );
    failures.extend(
        directive_review_run(
            "artifact-head-neutral",
            "Vou te pedir um relatório: os títulos e as legendas devem ficar em que idioma?",
            &[
                ("positive", "A usuária pede que o relatório escrito pelo assistente, incluindo títulos e legendas, fique em sueco."),
                ("foreign", "A usuária pede que o roteiro escrito pelo assistente, incluindo títulos e legendas, fique em sueco."),
            ],
            &["positive"],
        )
        .await,
    );
    assert!(
        failures.is_empty(),
        "colon-framed code request: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_review_question_without_mark_does_not_certify_foreign_executor() {
    let mut failures = Vec::new();
    for (tag, query, foreign, positive) in [
        (
            "en",
            "In which language should you reply to me",
            "The translator should reply in Finnish.",
            "User wants the assistant to reply in Finnish.",
        ),
        (
            "pt",
            "Em que língua você deve me responder",
            "O tradutor deve responder em italiano.",
            "A usuária quer que o assistente responda em italiano.",
        ),
    ] {
        for (mark, q) in [("bare", query.to_string()), ("marked", format!("{query}?"))] {
            failures.extend(
                directive_review_run(
                    &format!("f3-{tag}-{mark}-foreign"),
                    &q,
                    &[("foreign", foreign)],
                    &[],
                )
                .await,
            );
            failures.extend(
                directive_review_run(
                    &format!("f3-{tag}-{mark}-positive"),
                    &q,
                    &[("positive", positive)],
                    &["positive"],
                )
                .await,
            );
        }
    }
    assert!(
        failures.is_empty(),
        "unmarked question executor: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_time_alternatives_keep_notation_and_domain_boundaries() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for (label, query, positive, rejected, executor) in [
        (
            "notation-only",
            "Should you write times for me as 9 AM or as 09:00?",
            "User wants times in 24-hour format, never AM/PM.",
            "am pm",
            "User wants times in 24-hour format and translators as executors, never AM/PM.",
        ),
        (
            "notation-coordination",
            "Should you write times for me as 9 AM or as 09:00?",
            "User wants times in 16-hour format and intervals as HH:MM, never AM/PM or month-first dates.",
            "am pm or month-first dates",
            "User wants times in 16-hour format and couriers as reviewers, never AM/PM or month-first dates.",
        ),
        (
            "portuguese-format",
            "Você deve escrever os horários para mim como 09:00 ou como 9 AM?",
            "A usuária quer horários no formato de 12 horas, nunca no formato de 24 horas.",
            "no formato de 24 horas",
            "A usuária quer horários no formato de 12 horas e tradutores como executores, nunca no formato de 24 horas.",
        ),
        (
            "portuguese-neutral",
            "Você deve escrever os horários para mim como 09:00 ou como 9 AM?",
            "A usuária quer horários no formato de 16 horas, nunca no formato de 32 horas.",
            "no formato de 32 horas",
            "A usuária quer horários no formato de 16 horas e mensageiros como revisores, nunca no formato de 32 horas.",
        ),
    ] {
        let pt = label.starts_with("portuguese");
        let negative = positive.replacen(
            if pt { "quer" } else { "wants" },
            if pt { "não quer" } else { "does not want" },
            1,
        );
        let foreign = positive.replacen(
            if pt { "A usuária" } else { "User" },
            if pt { "A visitante" } else { "The visitor" },
            1,
        );
        let conditional = format!(
            "{} {}.",
            positive.trim_end_matches('.'),
            if pt { "se disponível" } else { "if available" }
        );
        let interrogative = format!("{}?", positive.trim_end_matches('.'));
        let quoted = format!("\"{positive}\"");
        let malformed = if pt {
            positive.replace(rejected, "no formato de se disponível")
        } else {
            positive.replace("AM/PM", "AM PM")
        };
        let cases = [
            ("positive", positive),
            ("negative", negative.as_str()),
            ("foreign", foreign.as_str()),
            ("conditional", conditional.as_str()),
            ("interrogative", interrogative.as_str()),
            ("quoted", quoted.as_str()),
            ("malformed", malformed.as_str()),
            ("executor", executor),
        ];
        for (id, text) in cases {
            let expected = if id == "positive" { vec![id] } else { vec![] };
            failures.extend(
                directive_review_run(&format!("{label}-{id}"), query, &[(id, text)], &expected)
                    .await,
            );
            executed += 1;
        }
        failures.extend(
            directive_review_run(&format!("{label}-pool"), query, &cases, &["positive"]).await,
        );
        executed += 1;
    }
    assert_eq!(executed, 36);
    assert!(failures.is_empty(), "time alternatives: {failures:?}");
}

#[tokio::test]
async fn directive_relation_review_format_rule_does_not_admit_introduced_executor() {
    let query = "Should you write times for me as 2 PM or as 14:00?";
    let mut failures = Vec::new();
    failures.extend(
        directive_review_run(
            "f4-introduced-executor",
            query,
            &[(
                "executor",
                "User wants times in 12-hour format and translators as executors.",
            )],
            &[],
        )
        .await,
    );
    failures.extend(
        directive_review_run(
            "f4-foreign-executor",
            query,
            &[("foreign", "The visitor wants times in 12-hour format and dates as DD/MM/YYYY, never 24-hour format.")],
            &[],
        )
        .await,
    );
    failures.extend(
        directive_review_run(
            "f4-original-positive",
            query,
            &[(
                "positive",
                "User wants times in 12-hour format and dates as DD/MM/YYYY, never 24-hour format.",
            )],
            &["positive"],
        )
        .await,
    );
    for (label, text, expected) in [
        (
            "notation-neutral",
            "User wants times in 16-hour format and intervals as HH:MM, never 32-hour format.",
            &["candidate"][..],
        ),
        (
            "role-neutral",
            "User wants times in 16-hour format and couriers as reviewers.",
            &[][..],
        ),
        (
            "conditional-notation",
            "User wants times in 16-hour format and intervals as HH/MM if available.",
            &[][..],
        ),
    ] {
        failures.extend(directive_review_run(label, query, &[("candidate", text)], expected).await);
    }
    assert!(failures.is_empty(), "format rule vs executor: {failures:?}");
}

// The exact English question has authentic vectors in both frozen caches.
// Synthetic candidates deliberately have NO vectors: isolated runs exercise
// lexical candidate evidence with a cached query, not cached candidate recall.
// Pooled runs also rank one unchanged, genuinely cached foreign-domain memory.
// Neither run supplies missing vectors for the original PT/9-AM questions.
// PT12/no-formato controls remain in the unchanged lexical test above: their
// positive has neither an exact cached question nor an exact cached candidate
// vector, and cannot be forced into this English query's ranked pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directive_relation_time_controls_with_authentic_cached_query_and_unembedded_candidates() {
    let query = "Should you write times for me as 2 PM or as 14:00?";
    let mut failures = Vec::new();
    let mut executed = 0;
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
            .map(|raw| serde_json::from_str(raw).unwrap())
            .unwrap_or_else(|| json!({"items":{},"model":""}));
        let model = cache["model"].as_str().unwrap();
        let query_vector: Option<Vec<f32>> = raw.map(|_| {
            let item = &cache["items"]["q-evt-time-format"];
            assert_eq!(item["text"], query);
            let vector: Vec<f32> = serde_json::from_value(item["v"].clone()).unwrap();
            assert_eq!(vector.len(), cache["dims"].as_u64().unwrap() as usize);
            assert!(vector.iter().all(|n| n.is_finite()));
            vector
        });
        if raw.is_some() {
            for missing in [
                "Should you write times for me as 9 AM or as 09:00?",
                "Você deve escrever os horários para mim como 09:00 ou como 9 AM?",
            ] {
                assert!(cache["items"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|v| v["text"] != missing));
                println!(
                    "{}",
                    json!({"diagnostic":"directive-time-cache-limit","lane":lane,
                    "missing_exact_query_vector":missing,"substituted_vector":false})
                );
            }
        }
        for (family, positive, executor) in [
            (
                "english-twelve",
                "User wants times in 12-hour format and dates as DD/MM/YYYY, never 24-hour format.",
                "User wants times in 12-hour format and translators as executors, never 24-hour format.",
            ),
            (
                "english-notation",
                "User wants times in 24-hour format, never AM/PM.",
                "User wants times in 24-hour format and translators as executors, never AM/PM.",
            ),
        ] {
            let negative = positive.replacen("wants", "does not want", 1);
            let foreign = positive.replacen("User", "The visitor", 1);
            let conditional = format!("{} if available.", positive.trim_end_matches('.'));
            let question = format!("{}?", positive.trim_end_matches('.'));
            let quoted = format!("\"{positive}\"");
            let malformed = if positive.contains("AM/PM") {
                positive.replace("AM/PM", "AM PM")
            } else {
                positive.replace("12-hour", "if-hour")
            };
            let inverse = "The assistant wants the user to write times in 12-hour format.";
            let cases = [
                ("positive", positive), ("negative", negative.as_str()),
                ("foreign", foreign.as_str()), ("conditional", conditional.as_str()),
                ("question", question.as_str()), ("quoted", quoted.as_str()),
                ("malformed", malformed.as_str()), ("inverse", inverse),
                ("executor", executor),
            ];
            for case_index in 0..=cases.len() {
                let pooled = case_index == cases.len();
                let pool = if pooled { &cases[..] } else { &cases[case_index..case_index + 1] };
                let expected: Vec<&str> = pool.iter()
                    .filter_map(|(id, _)| (*id == "positive").then_some(*id)).collect();
                let dir = tempfile::tempdir().unwrap();
                let boundary = super::cached_boundary::boundary(dir.path(), cache.clone()).await;
                let deps = &boundary.deps;
                let mut cfg = (**deps.config.read().await).clone();
                cfg.embeddings_enabled = raw.is_some();
                cfg.embedding_model = model.into();
                *deps.config.write().await = Arc::new(cfg);
                let store = deps.store().await;
                let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
                let mut memories = pool.to_vec();
                let anchor = &cache["items"]["evt-pref-code-english"];
                if pooled && raw.is_some() {
                    memories.push(("foreign-domain-anchor", anchor["text"].as_str().unwrap()));
                }
                for (id, text) in &memories {
                    bank.commit(Memory {
                        id: (*id).into(), text: (*text).into(),
                        entities: Vec::new(), tags: Vec::new(),
                        confidence: crate::types::Confidence::Stated,
                        corroboration: 0, pinned: false, source: None,
                        created_at: now_ms(), updated_at: now_ms(), stated_at: None,
                        invalid_at: None, superseded_by: None, revision: 0,
                    }).await.unwrap();
                    if *id == "foreign-domain-anchor" {
                        let vector: Vec<f32> = serde_json::from_value(anchor["v"].clone()).unwrap();
                        assert_eq!(vector.len(), query_vector.as_ref().unwrap().len());
                        assert!(vector.iter().all(|n| n.is_finite()));
                        bank.set_vector(id, model, text, vector).await.unwrap();
                    } else if raw.is_some() {
                        assert!(cache["items"].as_object().unwrap().values().all(|v| v["text"] != *text),
                            "synthetic candidate unexpectedly has an authentic vector");
                    }
                }
                assert_eq!(bank.vector_count().await, usize::from(pooled && raw.is_some()));
                bank.set_rule("standing", "Preserve the permanent safety rule.").unwrap();
                let (ranked, evidence) = bank.recall_evidence(
                    query, query_vector.as_deref(), model, usize::MAX, 30, false, 0.0).await;
                let reasons = admission_reasons(&ranked, &evidence);
                for (id, _) in &memories {
                    let supported = reasons.get(*id).is_some_and(|reason| reason.supported());
                    if supported != expected.contains(id) {
                        failures.push(format!("{lane}/{family}/{case_index}/{id}: pre-select {supported}"));
                    }
                    if *id != "foreign-domain-anchor" {
                        assert!(evidence.get(*id).is_none_or(|e| !e.vector),
                            "an unembedded synthetic candidate must not claim vector evidence");
                    }
                }
                let session = format!("time-control-{lane}-{family}-{case_index}");
                let response = hook(deps, &session,
                    json!([{"role":"user","content":query}]), context(&session,"direct",query)).await;
                if ids(&response) != json!(expected) {
                    failures.push(format!("{lane}/{family}/{case_index}: selection {}", ids(&response)));
                }
                rules_preserved(&response);
                let expected_calls = if raw.is_some() { vec![query.to_owned()] } else { vec![] };
                assert_eq!(*boundary.calls.lock().unwrap(), expected_calls,
                    "cached query must traverse the exact-text SDK boundary without cache miss");
                println!("{}", json!({"diagnostic":"directive-time-cached-query-controls",
                    "lane":lane,"family":family,"case":case_index,"pooled":pooled,
                    "query_vector_authentic":raw.is_some(),"synthetic_candidate_vectors":0,
                    "authentic_foreign_domain_vectors":usize::from(pooled && raw.is_some()),
                    "expected":expected,"selected":ids(&response),
                    "preselect":memories.iter().map(|(id,_)|json!({"id":id,
                        "supported":reasons.get(*id).is_some_and(|reason|reason.supported()),
                        "directive":evidence.get(*id).map(|e|&e.target_directive)})).collect::<Vec<_>>() }));
                executed += 1;
            }
        }
    }
    assert_eq!(executed, 60);
    assert!(
        failures.is_empty(),
        "cached-query time controls: {failures:?}"
    );
}

// These exact questions and candidates have no authentic vectors in either
// frozen cache. Exercise the production cache-miss -> nonembedding boundary,
// not semantic cached recall; never substitute another text's vector. The
// earlier sixty cached-query controls remain separate and unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directive_relation_time_missing_vectors_preserve_explicit_nonembedding_controls() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for (lane, raw) in [
        ("lexical", None),
        (
            "cached-a-miss-nonembedding",
            Some(include_str!("../recall_quality/vectors.json")),
        ),
        (
            "cached-b-miss-nonembedding",
            Some(include_str!("../recall_quality/vectors-minilm.json")),
        ),
    ] {
        let cache: Value = raw
            .map(|raw| serde_json::from_str(raw).unwrap())
            .unwrap_or_else(|| json!({"items":{},"model":""}));
        let model = cache["model"].as_str().unwrap();
        for (family, query, positive, executor, portuguese) in [
            (
                "portuguese-twelve",
                "Você deve escrever os horários para mim como 09:00 ou como 9 AM?",
                "A usuária quer horários no formato de 12 horas, nunca no formato de 24 horas.",
                "A usuária quer horários no formato de 12 horas e tradutores como executores, nunca no formato de 24 horas.",
                true,
            ),
            (
                "portuguese-neutral-format",
                "Você deve escrever os horários para mim como 09:00 ou como 9 AM?",
                "A usuária quer horários no formato de 16 horas, nunca no formato de 32 horas.",
                "A usuária quer horários no formato de 16 horas e mensageiros como revisores, nunca no formato de 32 horas.",
                true,
            ),
            (
                "english-am-pm",
                "Should you write times for me as 9 AM or as 09:00?",
                "User wants times in 24-hour format, never AM/PM.",
                "User wants times in 24-hour format and translators as executors, never AM/PM.",
                false,
            ),
        ] {
            let negative = positive.replacen(
                if portuguese { "quer" } else { "wants" },
                if portuguese { "não quer" } else { "does not want" }, 1);
            let foreign = positive.replacen(
                if portuguese { "A usuária" } else { "User" },
                if portuguese { "A visitante" } else { "The visitor" }, 1);
            let conditional = format!("{} {}.", positive.trim_end_matches('.'),
                if portuguese { "se disponível" } else { "if available" });
            let quoted = format!("\"{positive}\"");
            let question = format!("{}?", positive.trim_end_matches('.'));
            let malformed = if portuguese {
                positive.replace("no formato de 24 horas", "no formato de se disponível")
                    .replace("no formato de 32 horas", "no formato de se disponível")
            } else {
                positive.replace("AM/PM", "AM PM")
            };
            let inverse = if portuguese {
                "O assistente quer que a usuária escreva horários no formato de 12 horas."
            } else {
                "The assistant wants the user to write times in 24-hour format."
            };
            let cases = [
                ("positive", positive), ("negative", negative.as_str()),
                ("inverse", inverse), ("foreign", foreign.as_str()),
                ("conditional", conditional.as_str()), ("quoted", quoted.as_str()),
                ("question", question.as_str()), ("malformed", malformed.as_str()),
                ("executor", executor),
            ];
            // Check both missing ends, not merely the question. A cache key or
            // a vector from a related English sentence is not an exact witness.
            for text in std::iter::once(query).chain(cases.iter().map(|(_, text)| *text)) {
                assert!(cache["items"].as_object().unwrap().values()
                    .all(|item| item["text"] != text),
                    "this nonembedding control requires an exact cache miss: {text}");
            }
            for case_index in 0..=cases.len() {
                let pooled = case_index == cases.len();
                let pool = if pooled { &cases[..] } else { &cases[case_index..case_index + 1] };
                let expected: Vec<&str> = pool.iter()
                    .filter_map(|(id, _)| (*id == "positive").then_some(*id)).collect();
                let dir = tempfile::tempdir().unwrap();
                let boundary = super::cached_boundary::boundary(dir.path(), cache.clone()).await;
                let deps = &boundary.deps;
                let mut cfg = (**deps.config.read().await).clone();
                cfg.embeddings_enabled = raw.is_some();
                cfg.embedding_model = model.into();
                *deps.config.write().await = Arc::new(cfg);
                let store = deps.store().await;
                let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
                for (id, text) in pool {
                    bank.commit(Memory {
                        id: (*id).into(), text: (*text).into(),
                        entities: Vec::new(), tags: Vec::new(),
                        confidence: crate::types::Confidence::Stated,
                        corroboration: 0, pinned: false, source: None,
                        created_at: now_ms(), updated_at: now_ms(), stated_at: None,
                        invalid_at: None, superseded_by: None, revision: 0,
                    }).await.unwrap();
                }
                assert_eq!(bank.vector_count().await, 0);
                bank.set_rule("standing", "Preserve the permanent safety rule.").unwrap();
                // The mock returns cache_miss for the exact query; the hook
                // therefore reaches this same None/empty-model recall path.
                let (ranked, evidence) = bank.recall_evidence(
                    query, None, "", usize::MAX, 30, false, 0.0).await;
                let reasons = admission_reasons(&ranked, &evidence);
                for (id, _) in pool {
                    let supported = reasons.get(*id).is_some_and(|reason| reason.supported());
                    if supported != expected.contains(id) {
                        failures.push(format!("{lane}/{family}/{case_index}/{id}: pre-select {supported}"));
                    }
                    assert!(evidence.get(*id).is_none_or(|e| !e.vector && e.cosine.is_none() && e.z.is_none()),
                        "missing vectors must not become semantic evidence");
                }
                let session = format!("time-nonembedding-{lane}-{family}-{case_index}");
                let response = hook(deps, &session,
                    json!([{"role":"user","content":query}]), context(&session,"direct",query)).await;
                if ids(&response) != json!(expected) {
                    failures.push(format!("{lane}/{family}/{case_index}: selection {}", ids(&response)));
                }
                // Production annotates only successful semantic retrieval;
                // cache misses must not claim that annotation.
                assert!(response["annotations"].get("memory_retrieval").is_none());
                rules_preserved(&response);
                let expected_calls = if raw.is_some() { vec![query.to_owned()] } else { vec![] };
                assert_eq!(*boundary.calls.lock().unwrap(), expected_calls,
                    "only the exact missing query may reach the offline SDK boundary");
                let examined: Vec<&str> = pool.iter().map(|(id, _)| *id).collect();
                println!("{}", json!({"diagnostic":"directive-time-nonembedding-boundary",
                    "lane":lane,"family":family,"case":case_index,"pooled":pooled,
                    "query":query,"query_vector_authentic":false,"candidate_vector_count":0,
                    "cache_miss_expected":raw.is_some(),"semantic_cached_recall_measured":false,
                    "expected":expected,"selected":ids(&response),
                    "semantic_retrieval_annotation":response["annotations"]["memory_retrieval"],
                    "effective_recall_path":"nonembedding",
                    "preselect":directive_candidate_audit(&ranked,&evidence,&examined)}));
                executed += 1;
            }
        }
    }
    assert_eq!(executed, 90);
    assert!(
        failures.is_empty(),
        "nonembedding time controls: {failures:?}"
    );
}

// Preserve the original script/code positive and measure the missing negative
// before deduplication can hide it. These are unchanged acceptance oracles,
// not evidence that shared components identify two different artifact heads.
#[tokio::test]
async fn directive_relation_original_script_requires_independent_artifact_identity() {
    let corpus: Value =
        serde_json::from_str(include_str!("../recall_quality/golden.json")).unwrap();
    let original = corpus["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["id"] == "q-evt-code-lang")
        .unwrap();
    assert_eq!(original["expect"], json!(["evt-pref-code-english"]));
    let query = original["text"].as_str().unwrap();
    let fact = corpus["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["key"] == "evt-pref-code-english")
        .unwrap();
    let code = fact["text"].as_str().unwrap();
    let prose = "A usuária pede que a prosa escrita pelo assistente, incluindo nomes de variáveis e comentários, fique em inglês.";
    let mut failures = Vec::new();
    failures.extend(
        directive_review_run("script-code-positive", query, &[("code", code)], &["code"]).await,
    );
    failures.extend(
        directive_review_run("script-prose-isolated", query, &[("prose", prose)], &[]).await,
    );
    failures.extend(
        directive_review_run(
            "script-code-prose-pool",
            query,
            &[("code", code), ("prose", prose)],
            &["code"],
        )
        .await,
    );
    failures.extend(directive_review_run("report-foreign-isolated",
        "Vou te pedir um relatório: os títulos e as legendas devem ficar em que idioma?",
        &[("foreign", "A usuária pede que o roteiro escrito pelo assistente, incluindo títulos e legendas, fique em sueco.")],
        &[]).await);
    assert!(
        failures.is_empty(),
        "independent artifact identity: {failures:?}"
    );
}

// General type rules apply downward only and never identify artifact instances.
// Every negative is tested alone as well as in the same pool as the positive;
// shared components, language values and an assistant executor are insufficient.
#[tokio::test]
async fn directive_relation_general_subtype_rules_discriminate_artifacts_before_select() {
    let families = [
        (
            "script-code",
            "Vou te pedir um script: os nomes de variáveis e os comentários devem ficar em que idioma?",
            "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco.",
            "A usuária pede que a prosa escrita pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco.",
            "A usuária pede que o relatório escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco.",
        ),
        (
            "sonnet-poem",
            "For sonnet written by you, in which language should titles and captions be?",
            "User wants poem written by the assistant, including titles and captions, in Swedish.",
            "User wants haiku written by the assistant, including titles and captions, in Swedish.",
            "User wants report written by the assistant, including titles and captions, in Swedish.",
        ),
        (
            "haiku-text",
            "No haicai escrito por você, os títulos e as legendas devem ficar em que idioma?",
            "A usuária pede que o texto escrito pelo assistente, incluindo títulos e legendas, fique em sueco.",
            "A usuária pede que o soneto escrito pelo assistente, incluindo títulos e legendas, fique em sueco.",
            "A usuária pede que o código escrito pelo assistente, incluindo títulos e legendas, fique em sueco.",
        ),
        (
            "report-document",
            "Vou te pedir um relatório: os títulos e as legendas devem ficar em que idioma?",
            "A usuária pede que o documento escrito pelo assistente, incluindo títulos e legendas, fique em sueco.",
            "A usuária pede que a fatura escrita pelo assistente, incluindo títulos e legendas, fique em sueco.",
            "A usuária pede que o script escrito pelo assistente, incluindo títulos e legendas, fique em sueco.",
        ),
        (
            "invoice-text",
            "For invoice written by you, in which language should titles and captions be?",
            "User wants text written by the assistant, including titles and captions, in Swedish.",
            "User wants report written by the assistant, including titles and captions, in Swedish.",
            "User wants script written by the assistant, including titles and captions, in Swedish.",
        ),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (family, query, positive, sibling, foreign) in families {
        for (id, text, expected) in [
            ("positive", positive, true),
            ("sibling", sibling, false),
            ("foreign", foreign, false),
        ] {
            let required: &[&str] = if expected { &[id] } else { &[] };
            failures.extend(
                directive_review_run(
                    &format!("subtype-{family}-{id}"),
                    query,
                    &[(id, text)],
                    required,
                )
                .await,
            );
            executed += 1;
        }
        failures.extend(
            directive_review_run(
                &format!("subtype-{family}-pool"),
                query,
                &[
                    ("positive", positive),
                    ("sibling", sibling),
                    ("foreign", foreign),
                ],
                &["positive"],
            )
            .await,
        );
        executed += 1;
    }
    // The inverse cannot inherit a narrower rule. Unknown types may match
    // literally, but no unknown-to-known promotion is permitted.
    for (label, query, candidate, expected) in [
        ("inverse-code-script",
            "For code written by you, in which language should variable names and comments be?",
            "User wants script written by the assistant, including variable names and comments, in Swedish.", false),
        ("inverse-poem-sonnet",
            "For poem written by you, in which language should titles and captions be?",
            "User wants sonnet written by the assistant, including titles and captions, in Swedish.", false),
        ("unknown-foreign",
            "For widget written by you, in which language should titles and captions be?",
            "User wants text written by the assistant, including titles and captions, in Swedish.", false),
        ("unknown-literal",
            "For widget written by you, in which language should titles and captions be?",
            "User wants widget written by the assistant, including titles and captions, in Swedish.", true),
    ] {
        let required: &[&str] = if expected { &["candidate"] } else { &[] };
        failures.extend(directive_review_run(label, query, &[("candidate", candidate)], required).await);
        executed += 1;
    }
    assert_eq!(executed, 24);
    assert!(
        failures.is_empty(),
        "general directional subtype rules: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_subtype_does_not_bypass_governor_executor_domain_or_components() {
    let query =
        "Vou te pedir um script: os nomes de variáveis e os comentários devem ficar em que idioma?";
    let positive = "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco.";
    let negatives = [
        ("foreign-executor", "A usuária pede que o código escrito pelo colega, incluindo nomes de variáveis e comentários, fique em sueco."),
        ("negated-governor", "A usuária não pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco."),
        ("conditional", "Se a usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco."),
        ("quoted", "\"A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco.\""),
        ("question", "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco?"),
        ("different-components", "A usuária pede que o código escrito pelo assistente, incluindo títulos e legendas, fique em sueco."),
        ("unrelated-instance", "A usuária pede que o script da colega escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco."),
        ("embedded-context", "A usuária relata que a colega pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em sueco."),
    ];
    let mut failures = Vec::new();
    let mut executed = 0;
    for (label, negative) in negatives {
        failures.extend(directive_review_run(label, query, &[("negative", negative)], &[]).await);
        failures.extend(
            directive_review_run(
                &format!("subtype-boundary-{label}-pool"),
                query,
                &[("positive", positive), ("negative", negative)],
                &["positive"],
            )
            .await,
        );
        executed += 2;
    }
    assert_eq!(executed, 16);
    assert!(
        failures.is_empty(),
        "subtype grammatical boundaries: {failures:?}"
    );
}

// Separate authentic MiniLM fixture: never fills or replaces OpenAI256 cache-a.
// Every synthetic candidate and exact query has its own local vector. Existing
// cache/query-only/fallback tests above remain unchanged and separately labelled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directive_relation_time_authentic_local_minilm_both_ends_retrieve_and_inject() {
    let cases: Value = serde_json::from_str(include_str!("directive_minilm_inputs.json")).unwrap();
    let cache: Value = serde_json::from_str(include_str!("directive_minilm_vectors.json")).unwrap();
    // The independently recorded source manifest hashes these exact input bytes;
    // every query/candidate is also bound to an exact fixture text below.
    assert_eq!(
        cache["provenance"]["input_sha256"],
        "6943fa7c09d3242cfe9bba69775bd211657dbd731c28cca2f4d3652311c66333"
    );
    assert_eq!(
        cache["model"],
        "sentence-transformers/all-MiniLM-L6-v2@c9745ed1d9f2"
    );
    assert_eq!(cache["dims"], 384);
    assert_eq!(
        cache["provenance"]["model_sha256"],
        "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452"
    );
    assert_eq!(
        cache["provenance"]["tokenizer_sha256"],
        "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037"
    );
    assert_eq!(cache["provenance"]["both_ends"], true);
    assert_eq!(cache["provenance"]["normalization"], "L2 f32");
    let vector = |text: &str| -> Vec<f32> {
        let matches: Vec<_> = cache["items"]
            .as_object()
            .unwrap()
            .values()
            .filter(|item| item["text"] == text)
            .collect();
        assert_eq!(matches.len(), 1, "exact unique authentic input: {text}");
        let v: Vec<f32> = serde_json::from_value(matches[0]["v"].clone()).unwrap();
        assert_eq!(v.len(), 384);
        assert!(v.iter().all(|n| n.is_finite()));
        assert!((v.iter().map(|n| n * n).sum::<f32>() - 1.0).abs() < 0.00001);
        v
    };
    let model = cache["model"].as_str().unwrap();
    let mut failures = Vec::new();
    let mut executed = 0;
    for family in cases.as_array().unwrap() {
        let query = family["query"].as_str().unwrap();
        let qv = vector(query);
        let candidates = family["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 9);
        for case_index in 0..=candidates.len() {
            let pooled = case_index == candidates.len();
            let pool = if pooled {
                &candidates[..]
            } else {
                &candidates[case_index..case_index + 1]
            };
            let expected: Vec<&str> = pool
                .iter()
                .filter(|c| c["id"] == "positive")
                .map(|c| c["id"].as_str().unwrap())
                .collect();
            let dir = tempfile::tempdir().unwrap();
            let boundary = super::cached_boundary::boundary(dir.path(), cache.clone()).await;
            let deps = &boundary.deps;
            let mut cfg = (**deps.config.read().await).clone();
            cfg.embeddings_enabled = true;
            cfg.embedding_model = model.into();
            *deps.config.write().await = Arc::new(cfg);
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            for candidate in pool {
                let id = candidate["id"].as_str().unwrap();
                let text = candidate["text"].as_str().unwrap();
                bank.commit(Memory {
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
                })
                .await
                .unwrap();
                bank.set_vector(id, model, text, vector(text))
                    .await
                    .unwrap();
            }
            assert_eq!(bank.vector_count().await, pool.len());
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, Some(&qv), model, usize::MAX, 30, false, 0.0)
                .await;
            let reasons = admission_reasons(&ranked, &evidence);
            for candidate in pool {
                let id = candidate["id"].as_str().unwrap();
                let e = evidence
                    .get(id)
                    .expect("actual production candidate evidence");
                assert!(
                    e.vector && e.cosine.is_some_and(|n| n.is_finite()),
                    "both-end semantic retrieval required, not lexical fallback: {id}"
                );
                let supported = reasons.get(id).is_some_and(|reason| reason.supported());
                if supported != expected.contains(&id) {
                    failures.push(format!(
                        "{}/{case_index}/{id}: preselect={supported}",
                        family["id"]
                    ));
                }
            }
            let session = format!(
                "authentic-time-{}-{case_index}",
                family["id"].as_str().unwrap()
            );
            let response = hook(
                deps,
                &session,
                json!([{"role":"user","content":query}]),
                context(&session, "direct", query),
            )
            .await;
            if ids(&response) != json!(expected) {
                failures.push(format!(
                    "{}/{case_index}: selected={}",
                    family["id"],
                    ids(&response)
                ));
            }
            rules_preserved(&response);
            assert_eq!(
                *boundary.calls.lock().unwrap(),
                vec![query.to_owned()],
                "exact query through real SDK boundary; no miss or unexpected RPC"
            );
            println!(
                "{}",
                json!({"diagnostic":"directive-time-local-both-ends",
                "family":family["id"],"case":case_index,"pooled":pooled,
                "model":model,"query_vector_authentic":true,"candidate_vectors":pool.len(),
                "semantic_retrieval_measured":true,"lexical_fallback":false,
                "expected":expected,"selected":ids(&response),"provider_calls":0,
                "preselect":pool.iter().map(|c| { let id=c["id"].as_str().unwrap();
                    let e=&evidence[id]; json!({"id":id,"vector":e.vector,"cosine":e.cosine,
                    "directive":e.target_directive,"supported":reasons.get(id).is_some_and(|r|r.supported())})
                }).collect::<Vec<_>>() })
            );
            executed += 1;
        }
    }
    assert_eq!(executed, 40);
    assert!(
        failures.is_empty(),
        "authentic both-end controls: {failures:?}"
    );
}

#[tokio::test]
async fn directive_relation_subtypes_keep_neutral_components_values_and_inverse_separate() {
    let mut failures = Vec::new();
    for (family, query, positive, sibling, inverse_query, inverse) in [
        ("poem-neutral", "For sonnet written by you, in which language should headings and footnotes be?",
         "User wants poem written by the assistant, including headings and footnotes, in Finnish.",
         "User wants haiku written by the assistant, including headings and footnotes, in Finnish.",
         "For poem written by you, in which language should headings and footnotes be?",
         "User wants sonnet written by the assistant, including headings and footnotes, in Finnish."),
        ("document-neutral", "No relatório escrito por você, os cabeçalhos e os rodapés devem ficar em que idioma?",
         "A usuária pede que o documento escrito pelo assistente, incluindo cabeçalhos e rodapés, fique em finlandês.",
         "A usuária pede que a fatura escrita pelo assistente, incluindo cabeçalhos e rodapés, fique em finlandês.",
         "No documento escrito por você, os cabeçalhos e os rodapés devem ficar em que idioma?",
         "A usuária pede que o relatório escrito pelo assistente, incluindo cabeçalhos e rodapés, fique em finlandês."),
    ] {
        failures.extend(directive_review_run(&format!("{family}-positive"), query,
            &[("positive",positive)], &["positive"]).await);
        failures.extend(directive_review_run(&format!("{family}-sibling"), query,
            &[("sibling",sibling)], &[]).await);
        failures.extend(directive_review_run(&format!("{family}-pool"), query,
            &[("positive",positive),("sibling",sibling)], &["positive"]).await);
        failures.extend(directive_review_run(&format!("{family}-inverse"), inverse_query,
            &[("inverse",inverse)], &[]).await);
    }
    assert!(
        failures.is_empty(),
        "neutral subtype controls: {failures:?}"
    );
}

#[tokio::test]
async fn identity_name_witness_does_not_supply_team_quantity_or_unwitnessed_age() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
    bank.set_rule("standing", "Preserve the permanent safety rule.")
        .unwrap();
    let year = chrono::Utc::now().format("%Y").to_string();
    let records = [
        identity_relations::fact("name", "User's partner's name is Laleh."),
        identity_relations::fact(
            "team",
            "User is a senior acoustics engineer on the astronomy team at Willow Cooperative.",
        ),
        identity_relations::fact("age", &format!("User turned 33 years old in April {year}.")),
        identity_relations::fact("unknown", "User's sister's name is unknown."),
    ];
    for record in &records {
        bank.commit(record.clone()).await.unwrap();
    }
    for (n, (query, expected)) in [
        ("What's my partner's name?", json!(["name"])),
        ("How many people sit on the astronomy team?", json!([])),
        ("when is my birthday?", json!([])),
        ("What's my sister's name?", json!([])),
    ]
    .into_iter()
    .enumerate()
    {
        let session = format!("name-without-count-{n}");
        let (hits, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        let selected = select(hits, &evidence, &HashSet::new(), 6, query);
        assert_eq!(
            json!(selected
                .iter()
                .map(|(m, _)| m.id.as_str())
                .collect::<Vec<_>>()),
            expected
        );
        let response = hook(
            &deps,
            &session,
            json!([{"role":"user","content":query}]),
            context(&session, "direct", query),
        )
        .await;
        assert_eq!(ids(&response), expected, "{query}");
        rules_preserved(&response);
    }
    for record in records {
        assert_eq!(bank.get(&record.id).await.unwrap(), record);
    }
}

#[test]
fn relevance_contract_preference_context_never_certifies_unstated_types_or_foreign_roles() {
    use crate::store::query_projection::life_evidence;
    let book = "User's favorite book is The Silver Estuary by Ines Duarte.";
    for query in [
        "What is my favorite novel?",
        "Qual romance peruano eu mais amo?",
        "Qual livro eu prefiro?",
    ] {
        let proof = life_evidence(query, book).unwrap();
        assert_eq!(proof["matched"], true);
        assert_eq!(proof["certifies_work_is_novel"], false);
        assert_eq!(proof["certifies_work_nationality"], false);
        assert_eq!(proof["value"], "the silver estuary");
    }
    let style = "User prefers concise answers from you.";
    for query in [
        "Do I prefer concise answers or detailed explanations from you?",
        "Do I prefer detailed explanations or concise answers from you?",
    ] {
        let proof = life_evidence(query, style).unwrap();
        assert_eq!(proof["matched"], true);
        assert_eq!(proof["output_role"], "assistant_answer");
        assert!(proof["owner_bytes"].is_array());
        assert!(proof["governor_bytes"].is_array());
        assert!(proof["output_role_bytes"].is_array());
    }
    for query in [
        "What is the visitor's favorite novel?",
        "If I read novels, what is my favorite novel?",
        "User quotes: What is my favorite novel?",
        "Is my favorite novel Colombian?",
        "Who wrote my favorite novel?",
        "Which format is my favorite book in?",
        "Do you prefer short answers or long explanations from me?",
        "Does the visitor prefer short answers or long explanations from you?",
        "If I asked, would I prefer short answers or long explanations from you?",
        "User quotes: Do I prefer short answers or long explanations from you?",
        "Which language should you use for code comments?",
        "Which time format should you use?",
    ] {
        assert!(life_evidence(query, book).is_none(), "book {query}");
        assert!(life_evidence(query, style).is_none(), "style {query}");
    }
}
