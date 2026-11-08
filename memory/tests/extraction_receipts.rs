//! Real temporary persistent Store receipts; no engine, embeddings or model.
use memory::{
    store::Store,
    types::{Confidence, Memory, Provenance},
};

fn record() -> Memory {
    Memory {
        id: "synthetic-record".into(),
        text: "Amber instruments require copper calibration.".into(),
        entities: vec!["fixture".into()],
        tags: vec![],
        confidence: Confidence::Extracted,
        corroboration: 0,
        pinned: false,
        source: Some(Provenance {
            session_id: Some("synthetic-source".into()),
            entry_id: Some("entry".into()),
            agent: None,
        }),
        created_at: 1,
        updated_at: 1,
        invalid_at: None,
        superseded_by: None,
        revision: 0,
        stated_at: Some(1),
    }
}

#[tokio::test]
async fn receipt_reopen_and_stale_generation_cannot_repeat_revision_or_corroboration() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("private", None).await.unwrap();
    let original = record();
    bank.commit(original.clone()).await.unwrap();
    let stale_store = Store::open(dir.path().into()).unwrap();
    let stale = stale_store.bank("private").await.unwrap();
    let mut reinforced = original.clone();
    reinforced.corroboration = 1;
    reinforced.revision = 1;
    assert!(bank
        .extraction_expected(
            "source/bank/job/entry/effect",
            reinforced.clone(),
            Some(&original),
            None
        )
        .await
        .unwrap()
        .is_some());
    let bytes = std::fs::read(dir.path().join("private/memories.jsonl")).unwrap();
    assert!(stale
        .extraction_expected(
            "source/bank/job/entry/effect",
            reinforced.clone(),
            Some(&original),
            None
        )
        .await
        .unwrap()
        .is_none());
    let reopened = Store::open(dir.path().into()).unwrap();
    let bank = reopened.bank("private").await.unwrap();
    assert!(bank
        .extraction_seen("source/bank/job/entry/effect")
        .unwrap());
    assert!(!bank
        .extraction_seen("different-source/bank/job/entry/effect")
        .unwrap());
    assert_eq!(bank.get(&original.id).await.unwrap(), reinforced);
    assert_eq!(
        std::fs::read(dir.path().join("private/memories.jsonl")).unwrap(),
        bytes
    );
    let (other, _) = reopened.ensure_bank("other", None).await.unwrap();
    assert!(!other
        .extraction_seen("source/bank/job/entry/effect")
        .unwrap());
}

#[tokio::test]
async fn correction_and_receipt_fail_together_then_reopen_as_one_transition() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("private", None).await.unwrap();
    let original = record();
    bank.commit(original.clone()).await.unwrap();
    let mut replacement = record();
    replacement.id = "replacement".into();
    replacement.text = "Amber instruments now require silver calibration.".into();
    replacement.stated_at = Some(2);
    let path = dir.path().join("private/memories.jsonl");
    let before = std::fs::read(&path).unwrap();
    let blocker = dir.path().join("private/.memories.jsonl.tmp");
    std::fs::create_dir(&blocker).unwrap();
    assert!(bank
        .extraction_expected("correction", replacement.clone(), None, Some(&original))
        .await
        .is_err());
    assert!(!bank.extraction_seen("correction").unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(bank.get(&original.id).await.unwrap(), original);
    assert!(bank.get(&replacement.id).await.is_none());
    std::fs::rename(blocker, dir.path().join("synthetic-blocker-backup")).unwrap();
    assert!(bank
        .extraction_expected("correction", replacement.clone(), None, Some(&original))
        .await
        .unwrap()
        .is_some());
    let store = Store::open(dir.path().into()).unwrap();
    let bank = store.bank("private").await.unwrap();
    let old = bank.get(&original.id).await.unwrap();
    assert!(!old.is_live());
    assert_eq!(old.revision, 1);
    assert_eq!(old.superseded_by.as_deref(), Some("replacement"));
    assert_eq!(bank.get("replacement").await.unwrap(), replacement);
    assert!(bank
        .extraction_expected("correction", replacement, None, Some(&original))
        .await
        .unwrap()
        .is_none());
    assert_eq!(bank.list(10, 0, false, None).await.0.len(), 1);
}

#[tokio::test]
async fn rule_and_receipt_are_one_rename_and_api_edits_retain_replay_guard() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("private", None).await.unwrap();
    bank.set_rule("learned", "Original human instruction.")
        .unwrap();
    let path = bank.dir().join("rules/learned.md");
    let before = std::fs::read(&path).unwrap();
    let blocker = bank.dir().join("rules/.learned.md.tmp");
    std::fs::create_dir(&blocker).unwrap();
    let receipt = "session/bank/turn/entry/rule<!-- -->";
    assert!(bank
        .extraction_rule_expected(
            receipt,
            "learned",
            "Original human instruction.",
            "Learned replacement."
        )
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::rename(blocker, bank.dir().join("rules/synthetic-blocker-backup")).unwrap();
    assert!(bank
        .extraction_rule_expected(
            receipt,
            "learned",
            "Original human instruction.",
            "Learned replacement."
        )
        .unwrap());
    let published = std::fs::read(&path).unwrap();
    let reopened = Store::open(dir.path().into()).unwrap();
    let bank = reopened.bank("private").await.unwrap();
    assert_eq!(
        bank.list_rules().unwrap()[0].content,
        "Learned replacement."
    );
    assert!(!bank
        .extraction_rule_expected(
            receipt,
            "learned",
            "stale snapshot",
            "Repeated replacement."
        )
        .unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), published);
    bank.set_rule("learned", "").unwrap();
    assert!(bank.list_rules().unwrap().is_empty());
    let reopened = Store::open(dir.path().into()).unwrap();
    let bank = reopened.bank("private").await.unwrap();
    assert!(!bank
        .extraction_rule_expected(receipt, "learned", "", "Repeated replacement.")
        .unwrap());
    assert!(bank.list_rules().unwrap().is_empty());
    bank.set_rule("learned", "Concurrent human edit.").unwrap();
    assert!(bank
        .extraction_rule_expected("different-source", "learned", "", "Stale merge.")
        .is_err());
    assert_eq!(
        bank.list_rules().unwrap()[0].content,
        "Concurrent human edit."
    );
    assert!(bank
        .extraction_rule_expected(
            "different-source",
            "learned",
            "Concurrent human edit.",
            "Next source."
        )
        .unwrap());
}

#[tokio::test]
async fn plan_is_exact_source_bound_and_decision_cannot_be_replaced_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let plan = serde_json::json!({"items":[],"known":[],"layered":[]});
    store
        .save_extraction_plan("Session/A/[entry]", &plan)
        .unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    assert_eq!(
        store.extraction_plan("Session/A/[entry]").unwrap(),
        Some(plan.clone())
    );
    assert_eq!(store.extraction_plan("session/A/[entry]").unwrap(), None);
    assert!(store
        .save_extraction_plan("Session/A/[entry]", &serde_json::json!({"items":[1]}))
        .is_err());
    assert_eq!(
        store.extraction_plan("Session/A/[entry]").unwrap(),
        Some(plan)
    );
}
