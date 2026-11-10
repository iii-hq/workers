//! Real temporary persistent Store receipts; no engine, embeddings or model.

fn captured_range(id: &str) -> memory::store::PendingExtraction {
    memory::store::PendingExtraction::parse(serde_json::json!({
        "entries":[[id,"user","ORIGINAL_RANGE_CANARY retained verbatim",17]],
        "bank":"private","derived":false,"agent":null,"turn_id":"original-turn"
    }))
    .unwrap()
}

#[test]
fn retained_ranges_survive_reopen_and_never_evict_unprocessed_source_at_bound() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let original = captured_range("opaque-first");
    assert_eq!(original.attempts, 0); // legacy pending shape
    store
        .retain_extraction_range("session", original.clone(), false)
        .unwrap();
    store
        .acknowledge_extraction_range("session", &original)
        .unwrap();
    assert_eq!(
        store.extraction_ranges("session").unwrap()[0].pending,
        original
    );
    let stale = Store::open(dir.path().into()).unwrap();
    for n in 1..128 {
        store
            .retain_extraction_range("session", captured_range(&format!("opaque-{n}")), false)
            .unwrap();
    }
    assert!(store
        .retain_extraction_range("session", captured_range("overflow"), false)
        .is_err());
    let before = store.extraction_ranges("session").unwrap();
    assert_eq!(before.len(), 128);
    assert_eq!(before[0].pending, original);
    assert!(before.iter().all(|r| !r.completed));
    // Old Store generation must read disk, not overwrite newer receipts.
    stale
        .retain_extraction_range("session", original.clone(), true)
        .unwrap();
    let reopened = Store::open(dir.path().into()).unwrap();
    let ranges = reopened.extraction_ranges("session").unwrap();
    assert_eq!(ranges.len(), 128);
    assert!(ranges[0].completed);
    assert_eq!(ranges[0].pending, original);
    assert_eq!(&ranges[1..], &before[1..]);
    reopened
        .acknowledge_extraction_range("session", &original)
        .unwrap();
    assert_eq!(reopened.extraction_ranges("session").unwrap(), before[1..]);
}

#[test]
fn retained_schema_is_versioned_fail_closed_and_io_failure_keeps_pending_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let original = captured_range("opaque");
    let value = serde_json::to_value(&original).unwrap();
    store
        .set_pending_extraction("session", value.clone())
        .unwrap();
    store
        .retain_extraction_range("session", original.clone(), false)
        .unwrap();
    let key = serde_json::json!(["ranges", "session"]).to_string();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for b in key.bytes() {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let path = dir
        .path()
        .join(".extraction")
        .join(format!("{hash:016x}.json"));
    let before = std::fs::read(&path).unwrap();
    let blocker = path.with_file_name(format!(".{hash:016x}.json.tmp"));
    std::fs::create_dir(&blocker).unwrap();
    assert!(store
        .retain_extraction_range("session", original.clone(), true)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        store.pending_extraction("session").unwrap(),
        Some(value.clone())
    );
    std::fs::rename(&blocker, dir.path().join("preserved-blocker")).unwrap();
    let baseline: serde_json::Value = serde_json::from_slice(&before).unwrap();
    for mutation in [
        "version",
        "field",
        "pending",
        "state",
        "digest",
        "capture-version",
    ] {
        let mut invalid = baseline.clone();
        match mutation {
            "version" => invalid["plan"]["version"] = serde_json::json!(2),
            "field" => invalid["plan"]["authority"] = serde_json::json!(true),
            "pending" => {
                invalid["plan"]["ranges"][0]["pending"]["authority"] = serde_json::json!(true)
            }
            "digest" => {
                invalid["plan"]["ranges"][0]["pending"]["source_digest"] = serde_json::json!("bad")
            }
            "capture-version" => {
                invalid["plan"]["ranges"][0]["pending"]["version"] = serde_json::json!(99)
            }
            _ => invalid["plan"]["ranges"][0]["completed"] = serde_json::json!("unknown"),
        }
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let reopened = Store::open(dir.path().into()).unwrap();
        assert!(reopened.extraction_ranges("session").is_err());
        assert!(reopened
            .retain_extraction_range("session", original.clone(), true)
            .is_err());
        assert_eq!(
            reopened.pending_extraction("session").unwrap(),
            Some(value.clone())
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            serde_json::to_vec(&invalid).unwrap()
        );
    }
    let mut invalid = value;
    invalid["version"] = serde_json::json!(99);
    assert!(memory::store::PendingExtraction::parse(invalid).is_err());
}

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

#[tokio::test]
async fn stale_effect_records_terminal_conflict_without_overwriting_other_sessions() {
    for mutation in ["pin", "edit", "reinforce"] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        let (bank, _) = store.ensure_bank("private", None).await.unwrap();
        let original = record();
        bank.commit(original.clone()).await.unwrap();
        let other_session = Store::open(dir.path().into()).unwrap();
        let other_bank = other_session.bank("private").await.unwrap();
        let mut changed = original.clone();
        changed.revision += 1;
        match mutation {
            "pin" => changed.pinned = true,
            "edit" => changed.text = "Human edit keeps its independent calibration.".into(),
            "reinforce" => {
                changed.corroboration += 1;
                changed.source.as_mut().unwrap().session_id = Some("other-session".into());
            }
            _ => unreachable!(),
        }
        other_bank
            .commit_expected(changed.clone(), Some(&original))
            .await
            .unwrap();
        let mut replacement = record();
        replacement.id = "replacement".into();
        replacement.text = "Amber instruments now require silver calibration.".into();
        let receipt =
            serde_json::json!(["source-session", "private", ["e_request", "e_end"], 0]).to_string();
        assert!(bank
            .extraction_expected(&receipt, replacement.clone(), None, Some(&original))
            .await
            .unwrap()
            .is_none());
        let reopened = Store::open(dir.path().into()).unwrap();
        let bank = reopened.bank("private").await.unwrap();
        assert_eq!(bank.get(&original.id).await.unwrap(), changed);
        assert!(bank.get(&replacement.id).await.is_none());
        assert!(bank.extraction_seen(&receipt).unwrap());
        let path = bank.dir().join("memories.jsonl");
        let records: Vec<serde_json::Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[2],
            serde_json::json!({"extraction_receipt":receipt,"disposition":"expected_conflict"})
        );
        let before = std::fs::read(&path).unwrap();
        // The original stale plan remains a no-op after restart: neither a
        // late correction nor a second corroboration can be published.
        assert!(bank
            .extraction_expected(&receipt, replacement.clone(), None, Some(&original))
            .await
            .unwrap()
            .is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut unrelated = record();
        unrelated.id = "unrelated".into();
        unrelated.text = "Birch archives retain monthly invoices.".into();
        assert!(bank
            .extraction_expected("different-effect", unrelated.clone(), None, None)
            .await
            .unwrap()
            .is_some());
        assert_eq!(bank.get(&unrelated.id).await.unwrap(), unrelated);
        assert_eq!(bank.get(&original.id).await.unwrap(), changed);
        let (other, _) = reopened.ensure_bank("other", None).await.unwrap();
        assert!(!other.extraction_seen(&receipt).unwrap());
    }
}

#[tokio::test]
async fn only_typed_terminal_receipts_deduplicate_effects_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("private", None).await.unwrap();
    let path = bank.dir().join("memories.jsonl");
    let lines = [
        serde_json::json!({"extraction_receipt":"published"}),
        serde_json::json!({"extraction_receipt":"conflict","disposition":"expected_conflict"}),
        serde_json::json!({"extraction_receipt":"unknown","disposition":"unknown"}),
        serde_json::json!({"extraction_receipt":"extra","authority":true}),
        serde_json::json!({"extraction_receipt":null}),
    ]
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    std::fs::write(&path, lines.as_bytes()).unwrap();
    let reopened = Store::open(dir.path().into()).unwrap();
    let bank = reopened.bank("private").await.unwrap();
    assert!(bank.extraction_seen("published").unwrap());
    assert!(bank.extraction_seen("conflict").unwrap());
    assert!(!bank.extraction_seen("unknown").unwrap());
    assert!(!bank.extraction_seen("extra").unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), lines.as_bytes());
}

#[tokio::test]
async fn conflict_receipt_io_failure_is_not_swallowed_as_a_skipped_effect() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("private", None).await.unwrap();
    let original = record();
    bank.commit(original.clone()).await.unwrap();
    let mut changed = original.clone();
    changed.pinned = true;
    changed.revision += 1;
    bank.commit_expected(changed.clone(), Some(&original))
        .await
        .unwrap();
    let mut replacement = record();
    replacement.id = "replacement".into();
    let path = bank.dir().join("memories.jsonl");
    let before = std::fs::read(&path).unwrap();
    let blocker = bank.dir().join(".memories.jsonl.tmp");
    std::fs::create_dir(&blocker).unwrap();
    assert!(bank
        .extraction_expected("conflict", replacement.clone(), None, Some(&original))
        .await
        .is_err());
    assert!(!bank.extraction_seen("conflict").unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(bank.get(&original.id).await.unwrap(), changed);
    std::fs::rename(blocker, bank.dir().join("saved-blocker")).unwrap();
    assert!(bank
        .extraction_expected("conflict", replacement, None, Some(&original))
        .await
        .unwrap()
        .is_none());
    assert!(bank.extraction_seen("conflict").unwrap());
}
