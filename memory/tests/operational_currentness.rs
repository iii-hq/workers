//! Offline currentness through real store, recall and pre-generate paths.
//! Original factual labels and source text are read-only; no model or cache writes.
use memory::{
    config::WorkerConfig,
    deps::Deps,
    events::Emitter,
    hooks::{self, PreGenerateInput},
    store::Store,
    types::{Confidence, Memory, Provenance},
};
use serde_json::{json, Value};
use std::sync::Arc;

fn record(id: &str, text: &str, at: u64) -> Memory {
    Memory {
        id: id.into(),
        text: text.into(),
        entities: vec![],
        tags: vec![],
        confidence: Confidence::Stated,
        corroboration: 0,
        pinned: false,
        source: Some(Provenance {
            session_id: Some("synthetic".into()),
            entry_id: Some(id.into()),
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
fn deps(root: &std::path::Path) -> Arc<Deps> {
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
    Arc::new(Deps {
        hook_binding: Arc::default(),
        emitter: Arc::new(Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(
            Store::open(root.into()).unwrap(),
        ))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
            default_bank: "offline".into(),
            embeddings_enabled: false,
            extraction_enabled: false,
            recall_budget_tokens: 1200,
            ..Default::default()
        }))),
    })
}

#[tokio::test]
async fn currentness_nine_original_temporal_queries_preselect_before_limit_and_hook() {
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    bank.set_rule("standing", "Preserve this permanent synthetic rule.")
        .unwrap();
    let corpus: Value = serde_json::from_str(include_str!("recall_quality/golden.json")).unwrap();
    for fact in corpus["memories"].as_array().unwrap() {
        let at =
            memory::types::now_ms().saturating_sub(fact["age_days"].as_u64().unwrap() * 86_400_000);
        let mut m = record(
            fact["key"].as_str().unwrap(),
            fact["text"].as_str().unwrap(),
            at,
        );
        m.entities = serde_json::from_value(fact["entities"].clone()).unwrap_or_default();
        m.tags = serde_json::from_value(fact["tags"].clone()).unwrap_or_default();
        m.pinned = fact["pinned"].as_bool().unwrap_or(false);
        m.corroboration = fact["corroboration"].as_u64().unwrap_or(0) as u32;
        bank.commit(m).await.unwrap();
    }
    let before = std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap();
    let mut measured = 0;
    for q in corpus["queries"].as_array().unwrap().iter().filter(|q| {
        q["id"].as_str().unwrap().starts_with("q-upd-") && q["id"] != "q-upd-diet-recipe"
    }) {
        let id = q["id"].as_str().unwrap();
        let query = q["text"].as_str().unwrap();
        let old = q["stale"][0].as_str().unwrap();
        let new = q["expect"][0].as_str().unwrap();
        assert!(
            bank.get(old)
                .await
                .unwrap()
                .revalidated_by(&bank.get(new).await.unwrap()),
            "{id}"
        );
        let (hits, _) = bank
            .recall_evidence(query, None, "", 1, 30, false, 0.0)
            .await;
        assert!(
            hits.iter().all(|(m, _)| m.id != old),
            "stale preselection: {id}"
        );
        // History remains available; currentness never mutates source bytes.
        let (history, _) = bank
            .recall_evidence(query, None, "", 100, 30, true, 0.0)
            .await;
        assert!(history.iter().any(|(m, _)| m.id == old), "history: {id}");
        let response = memory::functions::recall::recall(
            &deps,
            serde_json::from_value(json!({"bank":"offline","query":query,"limit":100})).unwrap(),
        )
        .await
        .unwrap();
        assert!(
            response.memories.iter().any(|m| m.memory.id == new),
            "public recall missing current: {id}"
        );
        assert!(
            response.memories.iter().all(|m| m.memory.id != old),
            "public recall stale: {id}"
        );
        let input:PreGenerateInput=serde_json::from_value(json!({"session_id":format!("currentness-{id}"),"turn_id":format!("currentness-turn-{id}"),"step":0,
            "metadata":{"memory_bank":"offline"},"recall_context":{"version":1,"session_id":format!("currentness-{id}"),"turn_id":format!("currentness-turn-{id}"),"source":{"kind":"direct","entry_id":"question"},"query":{"entry_id":"question","text":query}},
            "generate":{"system_prompt":"offline","messages":[{"role":"user","content":query}]}})).unwrap();
        let out = hooks::pre_generate(&deps, input).await.unwrap();
        let annotations = out.annotations.unwrap();
        let ids: Vec<String> =
            serde_json::from_value(annotations.get("memory_ids").cloned().unwrap_or(json!([])))
                .unwrap();
        assert!(!ids.contains(&old.to_string()), "hook stale: {id}");
        assert!(
            ids.contains(&new.to_string()),
            "hook must independently admit current: {id}: {ids:?}"
        );
        measured += 1;
    }
    assert_eq!(measured, 9);
    assert_eq!(
        std::fs::read(dir.path().join("offline/memories.jsonl")).unwrap(),
        before
    );
    assert_eq!(
        bank.list_rules().unwrap()[0].content,
        "Preserve this permanent synthetic rule."
    );
}

#[test]
fn currentness_source_type_pinned_legacy_and_unrelated_guards() {
    let mut old = record(
        "old",
        "User's code editor is VS Code, with the Vim keybindings extension enabled.",
        1,
    );
    let mut new = record(
        "new",
        "User switched her code editor from VS Code to Neovim, configured with lazy.nvim.",
        2,
    );
    old.tags = vec!["editor".into()];
    new.tags = old.tags.clone();
    assert!(old.revalidated_by(&new));
    old.source = None;
    old.stated_at = None;
    assert!(old.revalidated_by(&new), "legacy stated compatibility");
    old.confidence = Confidence::Extracted;
    assert!(!old.revalidated_by(&new));
    old.confidence = Confidence::Stated;
    new.confidence = Confidence::Inferred;
    assert!(!old.revalidated_by(&new));
    new.confidence = Confidence::Stated;
    old.pinned = true;
    assert!(!old.revalidated_by(&new));
    old.pinned = false;
    new.text =
        "User switched her code editor from Emacs to Neovim, configured with lazy.nvim.".into();
    assert!(!old.revalidated_by(&new));
    new.text = "User switched her database from VS Code to Neovim.".into();
    assert!(!old.revalidated_by(&new));
    let durable = record("durable", "User prefers tea.", 0);
    assert!(!durable.revalidated_by(&new));
    let mut run_old = record(
        "run-old",
        "A usuária treina com o clube de corrida toda terça-feira de manhã, antes do trabalho.",
        1,
    );
    let mut run_new = record(
        "run-new",
        "A usuária agora treina com o clube de corrida às quintas-feiras à noite, e não mais às terças de manhã.",
        2,
    );
    run_old.tags = vec!["corrida".into()];
    run_new.tags = run_old.tags.clone();
    assert!(run_old.revalidated_by(&run_new));
    run_old.text = run_old.text.replace("terça-feira", "quarta-feira");
    assert!(
        !run_old.revalidated_by(&run_new),
        "must bind the previous day"
    );
    run_old.text = run_old.text.replace("quarta-feira", "terça-feira");
    run_new.text = "A usuária agora treina com o clube de corrida às quintas-feiras à noite; o evento terças de manhã não mais acontece.".into();
    assert!(
        !run_old.revalidated_by(&run_new),
        "incidental words are not a transition"
    );
}

#[test]
fn currentness_each_admission_is_independent_and_unsupported_old_cannot_certify_new() {
    use memory::{hooks::AdmissionReason, store::Evidence};
    use std::collections::{HashMap, HashSet};
    let old = record(
        "old",
        "User's code editor is VS Code, with the Vim keybindings extension enabled.",
        1,
    );
    let new = record(
        "new",
        "User switched her code editor from VS Code to Neovim, configured with lazy.nvim.",
        2,
    );
    let unknown = Evidence {
        matched: vec!["editor".into()],
        terms: 3,
        cosine: Some(0.99),
        z: Some(10.0),
        relevance: 100.0,
        ..Default::default()
    };
    let hits = vec![(old.clone(), 10.0), (new.clone(), 1.0)];
    let mut evidence = HashMap::from([
        (old.id.clone(), unknown.clone()),
        (new.id.clone(), unknown.clone()),
    ]);
    assert!(hooks::select(hits.clone(), &evidence, &HashSet::new(), 1, "editor").is_empty());
    let mut unrelated = new.clone();
    unrelated.id = "unrelated".into();
    unrelated.text = "User switched her database from SQLite to Postgres.".into();
    evidence.insert(unrelated.id.clone(), unknown.clone());
    assert!(hooks::select(
        vec![(old.clone(), 10.0), (unrelated, 1.0)],
        &evidence,
        &HashSet::new(),
        1,
        "editor"
    )
    .is_empty());
    let reasons = [
        AdmissionReason::RelationalTerms,
        AdmissionReason::RequestedAttribute,
        AdmissionReason::PersonalRelation,
        AdmissionReason::TargetRelation,
        AdmissionReason::NamedSubjectFact,
    ];
    for reason in reasons {
        let mut supported = unknown.clone();
        match reason {
            AdmissionReason::RelationalTerms => supported.terms = 1,
            AdmissionReason::RequestedAttribute => supported.attribute_match = true,
            AdmissionReason::PersonalRelation => supported.personal_match = true,
            AdmissionReason::TargetRelation => {
                supported.target_scope_required = true;
                supported.target_scope_matched = true;
                supported.terms = 1;
            }
            AdmissionReason::NamedSubjectFact => {
                supported.named_subject = true;
                supported.subject_question = true;
            }
            _ => unreachable!(),
        }
        evidence.insert(old.id.clone(), supported.clone());
        evidence.insert(new.id.clone(), unknown.clone());
        assert_eq!(hooks::admission_reasons(&hits, &evidence)[&old.id], reason);
        let selected = hooks::select(hits.clone(), &evidence, &HashSet::new(), 1, "editor");
        assert_eq!(
            selected
                .iter()
                .map(|(m, _)| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["old"],
            "unsupported new: {reason:?}"
        );
        evidence.insert(old.id.clone(), unknown.clone());
        evidence.insert(new.id.clone(), supported);
        let selected = hooks::select(hits.clone(), &evidence, &HashSet::new(), 1, "editor");
        assert_eq!(
            selected
                .iter()
                .map(|(m, _)| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new"],
            "independent current: {reason:?}"
        );
    }
}

#[tokio::test]
async fn currentness_preselection_keeps_current_when_old_would_fill_limit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    let mut old = record(
        "old",
        "User's code editor is VS Code, with the Vim keybindings extension enabled.",
        1,
    );
    let mut new = record(
        "new",
        "User switched her code editor from VS Code to Neovim, configured with lazy.nvim.",
        2,
    );
    old.tags = vec!["editor".into()];
    new.tags = old.tags.clone();
    old.corroboration = 100;
    bank.commit(old).await.unwrap();
    bank.commit(new).await.unwrap();
    let (hits, _) = bank
        .recall_evidence("Which code editor am I using?", None, "", 1, 30, false, 0.0)
        .await;
    assert_eq!(
        hits.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(),
        vec!["new"]
    );
}

#[tokio::test]
async fn currentness_store_does_not_hide_supported_detail_for_unsupported_transition() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    let mut old = record(
        "detail-old",
        "User's code editor is VS Code, with the Vim keybindings extension enabled.",
        1,
    );
    let mut new = record(
        "detail-new",
        "User switched her code editor from VS Code to Neovim, configured with lazy.nvim.",
        2,
    );
    old.tags = vec!["editor".into()];
    new.tags = old.tags.clone();
    assert!(old.revalidated_by(&new));
    bank.commit(old).await.unwrap();
    bank.commit(new).await.unwrap();
    let query = "Vim keybindings extension enabled";
    let (all, evidence) = bank
        .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
        .await;
    let reasons = hooks::admission_reasons(&all, &evidence);
    assert!(reasons["detail-old"].supported());
    assert!(!reasons.get("detail-new").is_some_and(|r| r.supported()));
    let (limited, _) = bank
        .recall_evidence(query, None, "", 1, 30, false, 0.0)
        .await;
    assert_eq!(limited[0].0.id, "detail-old");
    assert_eq!(
        hooks::select(all, &evidence, &Default::default(), 1, query)[0]
            .0
            .id,
        "detail-old"
    );
}

#[tokio::test]
async fn currentness_source_type_owner_and_contradiction_guards_before_limit() {
    for control in [
        "source",
        "empty-source",
        "type",
        "owner",
        "different-fact",
        "same-topic",
        "equal-date",
        "pinned",
        "legacy",
        "attested-extracted",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
        let mut old = record(
            "old",
            "User's code editor is VS Code, with the Vim keybindings extension enabled.",
            1,
        );
        let mut new = record(
            "new",
            "User switched her code editor from VS Code to Neovim, configured with lazy.nvim.",
            2,
        );
        old.tags = vec!["editor".into()];
        new.tags = old.tags.clone();
        old.corroboration = 100;
        match control {
            "source" => {
                new.confidence = Confidence::Extracted;
                new.source = None;
            }
            "empty-source" => {
                new.confidence = Confidence::Extracted;
                new.source.as_mut().unwrap().entry_id = Some(" ".into());
            }
            "type" => new.tags = vec!["database".into()],
            "owner" => {
                new.text = new
                    .text
                    .replace("User switched her", "User's colleague switched her")
            }
            "different-fact" => new.text = new.text.replace("from VS Code", "from Emacs"),
            "same-topic" => {
                new.text = "User now uses the code editor to write database migration notes.".into()
            }
            "equal-date" => {
                new.stated_at = Some(1);
                new.updated_at = 9;
            }
            "pinned" => old.pinned = true,
            "legacy" => {
                old.source = None;
                old.stated_at = None;
            }
            "attested-extracted" => {
                old.confidence = Confidence::Extracted;
                new.confidence = Confidence::Extracted;
            }
            _ => unreachable!(),
        }
        let expected_stale = matches!(control, "legacy" | "attested-extracted");
        assert_eq!(old.revalidated_by(&new), expected_stale, "{control}");
        bank.commit(old).await.unwrap();
        bank.commit(new).await.unwrap();
        let query = "Which code editor am I using?";
        let (all, evidence) = bank
            .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
            .await;
        assert_eq!(
            all.iter().any(|(m, _)| m.id == "old"),
            !expected_stale,
            "store: {control}"
        );
        let selected = hooks::select(all, &evidence, &Default::default(), 6, query);
        assert_eq!(
            selected.iter().any(|(m, _)| m.id == "old"),
            !expected_stale,
            "select: {control}"
        );
        let (limited, _) = bank
            .recall_evidence(query, None, "", 1, 30, false, 0.0)
            .await;
        assert_eq!(limited.len(), 1);
        assert_eq!(
            limited[0].0.id,
            if expected_stale { "new" } else { "old" },
            "before limit: {control}"
        );
    }
}

#[tokio::test]
async fn correction_atomic_rejects_stale_target_before_publishing_and_retries() {
    for change in ["edit", "pin", "retire"] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
        let old = record("old", "Synthetic original state", 1);
        let new = record("new", "Synthetic corrected state", 2);
        bank.commit(old.clone()).await.unwrap();
        let mut changed = old.clone();
        changed.revision += 1;
        match change {
            "edit" => changed.text.push_str(" edited"),
            "pin" => changed.pinned = true,
            _ => changed.invalid_at = Some(3),
        }
        bank.commit_expected(changed.clone(), Some(&old))
            .await
            .unwrap();
        let path = dir.path().join("offline/memories.jsonl");
        let before = std::fs::read(&path).unwrap();
        assert!(bank
            .correct_expected(&old, new.clone(), None)
            .await
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(bank.get("new").await.is_none());
        assert_eq!(bank.get("old").await.unwrap(), changed);
        if change == "edit" {
            let retired = bank
                .correct_expected(&changed, new.clone(), None)
                .await
                .unwrap();
            assert_eq!(retired.superseded_by.as_deref(), Some("new"));
            let after = std::fs::read(&path).unwrap();
            assert!(bank
                .correct_expected(&changed, new.clone(), None)
                .await
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), after);
            let reloaded = Store::open(dir.path().into()).unwrap();
            let b = reloaded.bank("offline").await.unwrap();
            assert_eq!(b.get("new").await.unwrap(), new);
            assert!(!b.get("old").await.unwrap().is_live());
        }
    }
}

#[tokio::test]
async fn correction_atomic_storage_failure_leaves_neither_side_published_then_retries() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    let old = record("old", "Synthetic original state", 1);
    let new = record("new", "Synthetic corrected state", 2);
    bank.commit(old.clone()).await.unwrap();
    let path = dir.path().join("offline/memories.jsonl");
    let before = std::fs::read(&path).unwrap();
    // A synthetic directory blocks temp creation without modifying the log.
    let blocker = dir.path().join("offline/.memories.jsonl.tmp");
    std::fs::create_dir(&blocker).unwrap();
    assert!(bank
        .correct_expected(&old, new.clone(), None)
        .await
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(bank.get("new").await.is_none());
    assert_eq!(bank.get("old").await.unwrap(), old);
    // Move the synthetic blocker aside; never remove user data.
    std::fs::rename(
        &blocker,
        dir.path().join("offline/synthetic-blocker-backup"),
    )
    .unwrap();
    bank.correct_expected(&old, new.clone(), None)
        .await
        .unwrap();
    assert_eq!(bank.get("new").await.unwrap(), new);
    assert!(!bank.get("old").await.unwrap().is_live());
}

#[tokio::test]
async fn currentness_recall_snapshot_stays_in_turn_and_next_turn_reads_correction() {
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(dir.path());
    let store = deps.store().await;
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    let mut old = record(
        "snapshot-old",
        "User's code editor is VS Code, with the Vim keybindings extension enabled.",
        1,
    );
    old.tags = vec!["editor".into()];
    bank.commit(old.clone()).await.unwrap();
    let query = "Which code editor am I using these days?";
    let input = |turn: &str, step: u64, messages: Value| -> PreGenerateInput {
        serde_json::from_value(json!({"session_id":"snapshot-session","turn_id":turn,"step":step,
            "metadata":{"memory_bank":"offline"},"recall_context":{"version":1,"session_id":"snapshot-session","turn_id":turn,"source":{"kind":"direct","entry_id":"question"},"query":{"entry_id":"question","text":query}},
            "generate":{"system_prompt":"offline","messages":messages}})).unwrap()
    };
    let first = hooks::pre_generate(
        &deps,
        input(
            "snapshot-turn-a",
            0,
            json!([{"role":"user","content":query}]),
        ),
    )
    .await
    .unwrap();
    let first_ids = first
        .annotations
        .as_ref()
        .unwrap()
        .get("memory_ids")
        .unwrap();
    assert_eq!(first_ids, &json!([old.id]));
    let mut new = record(
        "snapshot-new",
        "User switched her code editor from VS Code to Neovim, configured with lazy.nvim.",
        2,
    );
    new.tags = old.tags.clone();
    bank.correct_expected(&old, new.clone(), None)
        .await
        .unwrap();
    let held = first.mutations.unwrap().append_messages;
    let mut window = vec![json!({"role":"user","content":query})];
    window.extend(held);
    let same = hooks::pre_generate(&deps, input("snapshot-turn-a", 1, json!(window)))
        .await
        .unwrap();
    assert!(
        same.annotations.unwrap().get("memory_ids").is_none(),
        "later step must not re-recall"
    );
    let next = hooks::pre_generate(
        &deps,
        input(
            "snapshot-turn-b",
            0,
            json!([{"role":"user","content":query}]),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        next.annotations.unwrap().get("memory_ids"),
        Some(&json!([new.id]))
    );
}
