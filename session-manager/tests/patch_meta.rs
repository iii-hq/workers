//! `session::patch-meta` under concurrency: patches of different keys that
//! race each other all land, because the merge runs under the session's write
//! lock (a read-modify-write through `session::set-meta` loses all but one).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use session_manager::config::WorkerConfig;
use session_manager::functions::create::CreateRequest;
use session_manager::functions::patch_meta::PatchMetaRequest;
use session_manager::service::SessionService;
use session_manager::store::FsStore;

fn service(dir: &std::path::Path) -> Arc<SessionService> {
    let store = Arc::new(FsStore::new(dir).expect("open FsStore"));
    Arc::new(SessionService::new(store, &WorkerConfig::default()))
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_patches_of_different_keys_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let (created, _) = service
        .create(CreateRequest {
            title: Some("t".into()),
            metadata: Some(map(json!({ "owner": "u_1" }))),
            ..CreateRequest::default()
        })
        .await
        .unwrap();
    let session_id = created.session_id;

    let mut tasks = Vec::new();
    for index in 0..24 {
        let service = service.clone();
        let session_id = session_id.clone();
        tasks.push(tokio::spawn(async move {
            service
                .patch_meta(PatchMetaRequest {
                    session_id,
                    title: None,
                    description: None,
                    set: Some(map(json!({ format!("k{index}"): index }))),
                    unset: None,
                    touch: None,
                })
                .await
                .map(|(_, events)| events.len())
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().unwrap(), 1, "each patch emits once");
    }

    let (merged, _) = service
        .patch_meta(PatchMetaRequest {
            session_id: session_id.clone(),
            title: None,
            description: None,
            set: None,
            unset: None,
            touch: None,
        })
        .await
        .unwrap();
    let metadata = merged.meta.metadata.expect("metadata");
    assert_eq!(metadata.get("owner"), Some(&json!("u_1")));
    for index in 0..24 {
        assert_eq!(
            metadata.get(&format!("k{index}")),
            Some(&json!(index)),
            "patch {index} was lost"
        );
    }
}

#[tokio::test]
async fn two_racing_patches_of_one_key_leave_one_whole_value() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let (created, _) = service.create(CreateRequest::default()).await.unwrap();
    let patch = |value: Value| PatchMetaRequest {
        session_id: created.session_id.clone(),
        title: None,
        description: None,
        set: Some(map(json!({ "description_history": value }))),
        unset: None,
        touch: None,
    };
    let (a, b) = tokio::join!(
        service.patch_meta(patch(json!([{ "text": "a" }]))),
        service.patch_meta(patch(json!([{ "text": "b" }])))
    );
    a.unwrap();
    b.unwrap();
    let (meta, _) = service
        .patch_meta(PatchMetaRequest {
            session_id: created.session_id.clone(),
            title: None,
            description: None,
            set: None,
            unset: None,
            touch: None,
        })
        .await
        .unwrap();
    let history = meta.meta.metadata.unwrap()["description_history"].clone();
    assert!(
        history == json!([{ "text": "a" }]) || history == json!([{ "text": "b" }]),
        "{history}"
    );
}

/// chat-projects D50: a membership write (`touch: false`) keeps `updated_at`,
/// still emits once; the default bumps it; a no-op stays silent.
#[tokio::test]
async fn touch_false_keeps_updated_at_and_still_emits_once() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let (created, _) = service.create(CreateRequest::default()).await.unwrap();
    let before = created.meta.updated_at;
    let patch = |touch: Option<bool>, project: &str| PatchMetaRequest {
        session_id: created.session_id.clone(),
        title: None,
        description: None,
        set: Some(map(json!({ "project_id": project }))),
        unset: None,
        touch,
    };
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (quiet, events) = service
        .patch_meta(patch(Some(false), "prj_1"))
        .await
        .unwrap();
    assert_eq!(
        quiet.meta.updated_at, before,
        "touch: false keeps updated_at"
    );
    assert_eq!(events.len(), 1, "the write is still announced");
    assert_eq!(
        quiet.meta.metadata.as_ref().unwrap()["project_id"],
        json!("prj_1")
    );

    let (_, events) = service
        .patch_meta(patch(Some(false), "prj_1"))
        .await
        .unwrap();
    assert!(events.is_empty(), "a no-op stays silent");

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (touched, events) = service.patch_meta(patch(None, "prj_2")).await.unwrap();
    assert!(
        touched.meta.updated_at > before,
        "the default bumps updated_at"
    );
    assert_eq!(events.len(), 1);
}
