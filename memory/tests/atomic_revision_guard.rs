//! Supplied expected snapshots must actually guard a record revision.
use memory::{
    store::Store,
    types::{Confidence, Memory},
};
use serde_json::{json, Value};

fn record(id: &str) -> Memory {
    Memory {
        id: id.into(),
        text: "A stable synthetic fact.".into(),
        entities: vec![],
        tags: vec![],
        confidence: Confidence::Stated,
        corroboration: 0,
        pinned: false,
        source: None,
        created_at: 1,
        updated_at: 1,
        stated_at: Some(1),
        invalid_at: None,
        superseded_by: None,
        revision: 0,
    }
}

#[tokio::test]
async fn atomic_revision_projection_without_revision_cannot_authorize_any_write() {
    for invalid in [
        json!({"id":"b"}),
        json!({"text":"A stable synthetic fact."}),
        json!({"revision":null}),
        json!({"revision":-1}),
        json!({"revision":"0"}),
    ] {
        for guard_loser in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path().into()).unwrap();
            let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
            bank.commit(record("a")).await.unwrap();
            bank.commit(record("b")).await.unwrap();
            let path = dir.path().join("offline/memories.jsonl");
            let before = std::fs::read(&path).unwrap();
            let pair = if guard_loser {
                (Some(&invalid), None)
            } else {
                (None, Some(&invalid))
            };
            assert!(bank.supersede("b", "a", true, pair).await.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert!(bank.get("b").await.unwrap().is_live());
            assert_eq!(bank.get("a").await.unwrap().corroboration, 0);
        }
    }
}

#[tokio::test]
async fn atomic_revision_only_projection_rejects_interleaved_edit_and_allows_current_revision() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let (bank, _) = store.ensure_bank("offline", None).await.unwrap();
    bank.commit(record("a")).await.unwrap();
    bank.commit(record("b")).await.unwrap();
    let original = bank.get("b").await.unwrap();
    let mut edited = original.clone();
    edited.tags.push("new-detail".into());
    edited.revision += 1;
    bank.commit_expected(edited, Some(&original)).await.unwrap();
    let path = dir.path().join("offline/memories.jsonl");
    let before = std::fs::read(&path).unwrap();
    let stale = json!({"revision":0});
    assert!(bank
        .supersede("b", "a", true, (Some(&stale), None))
        .await
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let current: Value = json!({"revision":1});
    let winner = json!({"revision":0});
    bank.supersede("b", "a", true, (Some(&current), Some(&winner)))
        .await
        .unwrap();
    assert!(!bank.get("b").await.unwrap().is_live());
    assert_eq!(bank.get("a").await.unwrap().corroboration, 1);
}
