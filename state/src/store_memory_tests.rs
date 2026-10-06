//! Regression coverage for shared snapshots and cancellation-safe persistence.
use super::*;
use std::time::Duration;

fn directory() -> PathBuf {
    std::env::temp_dir().join(format!("state-memory-{}", uuid::Uuid::new_v4()))
}

// Explicit flush tests own the lifecycle; no immediate background tick races them.
fn manual_store(dir: &Path) -> KvStore {
    KvStore {
        store: Arc::new(RwLock::new(HashMap::new())),
        file_store_dir: Some(dir.to_path_buf()),
        dirty: Arc::new(RwLock::new(HashMap::new())),
        save_loop_stop: Arc::new(std::sync::Mutex::new(None)),
        flush_lock: Arc::new(tokio::sync::Mutex::new(())),
        default_interval: 60_000,
    }
}

fn read_legacy(dir: &Path, index: &str) -> IndexMap<String, Value> {
    let bytes = std::fs::read(dir.join(index_file_name(index))).unwrap();
    let value = rkyv::from_bytes::<KeyStorage, rkyv::rancor::Error>(&bytes).unwrap();
    serde_json::from_str(&value.0).unwrap()
}

#[tokio::test]
async fn shared_snapshot_survives_replacement_and_delete() {
    let store = in_memory_store();
    let initial = serde_json::json!({"payload":"x".repeat(65536)});
    store.set("s".into(), "a".into(), initial.clone()).await;
    store.set("s".into(), "b".into(), Value::Bool(true)).await;
    let snapshot = store.store.read().await["s"].clone();
    {
        let live = store.store.read().await;
        assert!(Arc::ptr_eq(&snapshot["a"], &live["s"]["a"]));
    }
    let result = store.set("s".into(), "a".into(), Value::Null).await;
    assert_eq!(result.old_value, Some(initial.clone()));
    assert_eq!(result.new_value, Value::Null);
    store.delete("s".into(), "a".into()).await;
    assert_eq!(snapshot["a"].as_ref(), &initial);
    assert_eq!(
        store.list("s".into()).await,
        vec![StateValue::from(Value::Bool(true))]
    );
    let dir = directory();
    persist_index_to_disk(&dir, "snapshot", &snapshot).unwrap();
    assert_eq!(read_legacy(&dir, "snapshot")["a"], initial);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn archive_is_byte_compatible_with_legacy_and_load_validates() {
    let dir = directory();
    let fixtures = [
        IndexMap::<String, Value>::new(),
        IndexMap::from([
            (
                "first".into(),
                serde_json::json!({"s":"中文\n\\\"", "n":18446744073709551615u64}),
            ),
            (
                "second".into(),
                serde_json::json!([null,true,-7,1.25,{"payload":"x".repeat(8192)}]),
            ),
        ]),
    ];
    for (i, data) in fixtures.into_iter().enumerate() {
        let legacy = rkyv::to_bytes::<rkyv::rancor::Error>(&KeyStorage(
            serde_json::to_string(&data).unwrap(),
        ))
        .unwrap();
        let shared: Scope = data
            .iter()
            .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
            .collect();
        let index = format!("scope:{i}/中文");
        persist_index_to_disk(&dir, &index, &shared).unwrap();
        assert_eq!(
            std::fs::read(dir.join(index_file_name(&index))).unwrap(),
            legacy.as_slice()
        );
        assert_eq!(read_legacy(&dir, &index), data);
        assert_eq!(load_store_from_dir(&dir)[&index], shared);
        std::fs::write(dir.join(index_file_name(&format!("legacy-{i}"))), legacy).unwrap();
        assert_eq!(load_store_from_dir(&dir)[&format!("legacy-{i}")], shared);
    }
    std::fs::write(dir.join("broken.bin"), b"broken").unwrap();
    assert!(!load_store_from_dir(&dir).contains_key("broken"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn moved_array_preserves_json_contract() {
    for items in [
        vec![],
        vec![Value::Null, serde_json::json!({"a":[1,true,"text"]})],
    ] {
        let old = serde_json::to_value(&items).unwrap();
        let moved = Value::Array(items);
        assert_eq!(old, moved);
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&moved).unwrap()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_flush_keeps_writer_exclusion_until_disk_work_finishes() {
    let dir = directory();
    let store = Arc::new(manual_store(&dir));
    store
        .set(
            "s".into(),
            "key".into(),
            serde_json::json!({"generation":0}),
        )
        .await;
    // Hold data access so the blocking flush is deterministically in flight.
    let mut blocked = store.store.write().await;
    let first = {
        let store = store.clone();
        tokio::spawn(async move { store.flush().await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while !store.dirty.read().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(
        store.flush_lock.try_lock().is_err(),
        "aborted caller must not release writer exclusion"
    );
    let mut second = {
        let store = store.clone();
        tokio::spawn(async move { store.flush().await })
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut second)
            .await
            .is_err()
    );
    blocked
        .get_mut("s")
        .unwrap()
        .insert("key".into(), Arc::new(serde_json::json!({"generation":1})));
    store
        .dirty
        .write()
        .await
        .insert("s".into(), DirtyOp::Upsert);
    // A hot-reconfigured loop shares the same exclusion, not just explicit flush.
    store.reconfigure(&serde_json::json!({"save_interval_ms":100}));
    drop(blocked);
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    store.flush().await.unwrap();
    assert_eq!(
        read_legacy(&dir, "s")["key"],
        serde_json::json!({"generation":1})
    );
    if let Some(stop) = store.save_loop_stop.lock().unwrap().take() {
        let _ = stop.send(true);
    }
    let _guard = store.flush_lock.lock().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_writes_reconfigure_and_final_flush_keep_latest_values() {
    let dir = directory();
    let store = Arc::new(manual_store(&dir));
    let mut tasks = tokio::task::JoinSet::new();
    for key in 0..4 {
        let store = store.clone();
        tasks.spawn(async move {
            for generation in 0..30 {
                store
                    .set(
                        "s".into(),
                        key.to_string(),
                        serde_json::json!({"generation":generation}),
                    )
                    .await;
                if generation % 7 == 0 {
                    store.flush().await.unwrap();
                }
            }
        });
    }
    store.reconfigure(&serde_json::json!({"save_interval_ms":100}));
    while let Some(task) = tasks.join_next().await {
        task.unwrap();
    }
    store.flush().await.unwrap();
    let values = read_legacy(&dir, "s");
    assert_eq!(values.len(), 4);
    assert!(values.values().all(|v| v["generation"] == 29));
    if let Some(stop) = store.save_loop_stop.lock().unwrap().take() {
        let _ = stop.send(true);
    }
    let _guard = store.flush_lock.lock().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_scope_does_not_prevent_other_snapshots_and_can_retry() {
    let dir = directory();
    let store = manual_store(&dir);
    std::fs::create_dir_all(dir.join(index_file_name("blocked"))).unwrap();
    store
        .set("blocked".into(), "k".into(), Value::Bool(true))
        .await;
    store
        .set("healthy".into(), "k".into(), Value::Bool(false))
        .await;
    assert!(store.flush().await.is_err());
    assert_eq!(read_legacy(&dir, "healthy")["k"], Value::Bool(false));
    assert!(store.dirty.read().await.contains_key("blocked"));
    std::fs::remove_dir(dir.join(index_file_name("blocked"))).unwrap();
    store.flush().await.unwrap();
    assert_eq!(read_legacy(&dir, "blocked")["k"], Value::Bool(true));
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn list_has_same_wire_json_and_keeps_old_values_after_mutation() {
    let store = in_memory_store();
    let old = serde_json::json!({"p":"x".repeat(65536)});
    store.set("s".into(), "k".into(), old.clone()).await;
    let snapshot = store.list("s".into()).await;
    assert!(Arc::ptr_eq(
        &snapshot[0].0,
        &store.store.read().await["s"]["k"]
    ));
    store.set("s".into(), "k".into(), Value::Null).await;
    let expected = Value::Array(vec![old]);
    assert_eq!(serde_json::to_value(&snapshot).unwrap(), expected);
    assert_eq!(
        serde_json::to_vec(&snapshot).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert_eq!(
        serde_json::to_value(store.list("missing".into()).await).unwrap(),
        serde_json::json!([])
    );
}

#[tokio::test]
async fn list_crosses_unchanged_sdk_into_async_handler_as_legacy_array() {
    use iii_sdk::iii::IntoAsyncHandler;
    let store = Arc::new(in_memory_store());
    store
        .set("s".into(), "k".into(), serde_json::json!({"id":1}))
        .await;
    let handler = (move |_: Value| {
        let store = store.clone();
        async move { Ok::<_, iii_sdk::Error>(Some(store.list("s".into()).await)) }
    })
    .into_handler();
    assert_eq!(
        handler(Value::Null, None).await.unwrap(),
        serde_json::json!([{"id":1}])
    );
}

#[tokio::test]
async fn last_response_drop_releases_replaced_and_deleted_values() {
    let store = in_memory_store();
    store
        .set(
            "s".into(),
            "a".into(),
            serde_json::json!({"generation":0,"payload":"a".repeat(65536)}),
        )
        .await;
    store
        .set(
            "s".into(),
            "b".into(),
            serde_json::json!({"generation":0,"payload":"b".repeat(65536)}),
        )
        .await;
    let first = store.list("s".into()).await;
    let weak: Vec<_> = first.iter().map(|v| Arc::downgrade(&v.0)).collect();
    let second = store.list("s".into()).await;
    // Mutation results own old JSON; their own lifetime is tested separately.
    let old_set = store.set("s".into(), "a".into(), Value::Null).await;
    let old_delete = store.delete("s".into(), "b".into()).await;
    assert!(weak.iter().all(|w| w.strong_count() == 2));
    assert_eq!(first[0].0["generation"], 0);
    assert_eq!(second[1].0["generation"], 0);
    drop(first);
    assert!(weak.iter().all(|w| w.strong_count() == 1));
    drop(second);
    assert!(
        weak.iter().all(|w| w.upgrade().is_none()),
        "no cache or hidden Arc should retain old records"
    );
    assert_eq!(old_set.old_value.unwrap()["generation"], 0);
    assert_eq!(old_delete.old_value.unwrap()["generation"], 0);
}

#[tokio::test]
async fn dropping_store_keeps_inflight_response_valid_until_last_owner() {
    let store = in_memory_store();
    store
        .set("s".into(), "a".into(), serde_json::json!({"id":1}))
        .await;
    let response = store.list("s".into()).await;
    let weak = Arc::downgrade(&response[0].0);
    drop(store);
    assert_eq!(weak.strong_count(), 1);
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::json!([{"id":1}])
    );
    drop(response);
    assert!(weak.upgrade().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_reader_drops_its_snapshot_without_pinning_old_records() {
    let store = Arc::new(in_memory_store());
    store
        .set(
            "s".into(),
            "a".into(),
            serde_json::json!({"payload":"x".repeat(65536)}),
        )
        .await;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let reader = {
        let store = store.clone();
        tokio::spawn(async move {
            let snapshot = store.list("s".into()).await;
            ready_tx.send(Arc::downgrade(&snapshot[0].0)).unwrap();
            std::future::pending::<()>().await;
            std::hint::black_box(&snapshot);
        })
    };
    let weak = ready_rx.await.unwrap();
    store.delete("s".into(), "a".into()).await;
    assert_eq!(weak.strong_count(), 1);
    reader.abort();
    assert!(reader.await.unwrap_err().is_cancelled());
    assert!(weak.upgrade().is_none());
    assert!(store.list("s".into()).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_list_conversion_and_writers_keep_each_record_coherent() {
    let store = Arc::new(in_memory_store());
    for id in 0..64 {
        store
            .set(
                "s".into(),
                format!("k{id:03}"),
                serde_json::json!({"id":id,"generation":0,"payload":"0".repeat(4096)}),
            )
            .await;
    }
    let old = store.list("s".into()).await;
    let weak: Vec<_> = old.iter().map(|v| Arc::downgrade(&v.0)).collect();
    let gate = Arc::new(tokio::sync::Barrier::new(5));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let store = store.clone();
        let gate = gate.clone();
        tasks.spawn(async move {
            gate.wait().await;
            for _ in 0..20 {
                let snapshot = store.list("s".into()).await;
                let array = serde_json::to_value(&snapshot).unwrap();
                assert_eq!(array.as_array().unwrap().len(), 64);
                for (id, value) in array.as_array().unwrap().iter().enumerate() {
                    assert_eq!(value["id"], id);
                    let generation = value["generation"].as_u64().unwrap() as u8;
                    assert!(
                        value["payload"]
                            .as_str()
                            .unwrap()
                            .bytes()
                            .all(|b| b == b'0' + generation)
                    );
                }
                tokio::task::yield_now().await;
            }
        });
    }
    gate.wait().await;
    for generation in 1..=3 {
        for id in 0..64 {
            store
                .set(
                    "s".into(),
                    format!("k{id:03}"),
                    serde_json::json!({"id":id,"generation":generation,
                "payload":((b'0'+generation as u8) as char).to_string().repeat(4096)}),
                )
                .await;
            tokio::task::yield_now().await;
        }
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(old.iter().all(|v| v["generation"] == 0));
    assert!(weak.iter().all(|w| w.strong_count() == 1));
    drop(old);
    assert!(weak.iter().all(|w| w.upgrade().is_none()));
    assert!(
        store
            .list("s".into())
            .await
            .iter()
            .all(|v| v["generation"] == 3)
    );
}

#[tokio::test]
async fn full_array_conversion_preserves_nulls_numbers_unicode_and_order() {
    let store = in_memory_store();
    let expected = vec![
        Value::Null,
        serde_json::json!([true, -1, 18446744073709551615u64, 1.25]),
        serde_json::json!({"unicode":"中文","escape":"\n\\\"","nested":{"array":[]}}),
    ];
    for (i, value) in expected.iter().enumerate() {
        store.set("s".into(), i.to_string(), value.clone()).await;
    }
    let owned = store.list("s".into()).await;
    let shared = store.list("s".into()).await;
    let legacy = serde_json::to_value(&owned).unwrap();
    let moved = Value::Array(
        owned
            .into_iter()
            .map(|v| Arc::unwrap_or_clone(v.0))
            .collect(),
    );
    let converted = serde_json::to_value(&shared).unwrap();
    assert_eq!(legacy, Value::Array(expected));
    assert_eq!(legacy, moved);
    assert_eq!(legacy, converted);
    assert_eq!(
        serde_json::to_vec(&legacy).unwrap(),
        serde_json::to_vec(&shared).unwrap()
    );
}

#[tokio::test]
async fn update_reuses_old_tree_unless_a_snapshot_still_owns_it() {
    let store = in_memory_store();
    let original = serde_json::json!({"payload":"x".repeat(65536),"count":0});
    store.set("s".into(), "key".into(), original.clone()).await;
    let pointer = {
        let map = store.store.read().await;
        map["s"]["key"]["payload"].as_str().unwrap().as_ptr() as usize
    };
    let result = store
        .update(
            "s".into(),
            "key".into(),
            vec![UpdateOp::increment("count", 1)],
        )
        .await;
    assert!(result.errors.is_empty());
    assert_eq!(
        result.old_value.as_ref().unwrap()["payload"]
            .as_str()
            .unwrap()
            .as_ptr() as usize,
        pointer
    );
    assert_eq!(result.old_value.unwrap(), original);
    assert_eq!(result.new_value["count"], 1);
    let snapshot = store.list("s".into()).await;
    let weak = Arc::downgrade(&snapshot[0].0);
    let result = store
        .update(
            "s".into(),
            "key".into(),
            vec![UpdateOp::increment("count", 1)],
        )
        .await;
    assert!(result.errors.is_empty());
    assert_eq!(snapshot[0].0["count"], 1);
    assert_eq!(result.old_value.unwrap()["count"], 1);
    assert_eq!(result.new_value["count"], 2);
    assert_eq!(weak.strong_count(), 1);
    drop(snapshot);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn materialized_get_and_list_remain_independent_and_in_order() {
    let store = in_memory_store();
    for (key, id) in [("z", 0), ("a", 1), ("m", 2)] {
        store
            .set(
                "s".into(),
                key.into(),
                serde_json::json!({"id":id,"generation":0}),
            )
            .await;
    }
    let first_read = store.get("s".into(), "z".into()).await.unwrap();
    let mut first = first_read.as_ref().clone();
    let reads = store.list("s".into()).await;
    let mut values: Vec<Value> = reads.iter().map(|v| v.as_ref().clone()).collect();
    assert_eq!(
        values
            .iter()
            .map(|v| v["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    first["generation"] = Value::from(99);
    values[0]["generation"] = Value::from(88);
    assert_eq!(
        store.get("s".into(), "z".into()).await.unwrap()["generation"],
        0
    );
    store.delete("s".into(), "z".into()).await;
    assert_eq!(first["generation"], 99);
    assert_eq!(values[0]["generation"], 88);
}

#[test]
fn clean_flush_does_not_require_a_blocking_thread() {
    // A busy blocking pool makes the old unconditional spawn_blocking time out.
    // The optimized empty path must finish on the executor without disk access.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let busy = tokio::task::spawn_blocking(move || {
            started_tx.send(()).unwrap();
            let _ = release_rx.recv();
        });
        started_rx.await.unwrap();
        let dir = directory();
        let store = manual_store(&dir);
        let result = tokio::time::timeout(Duration::from_millis(250), store.flush()).await;
        release_tx.send(()).unwrap();
        busy.await.unwrap();
        result
            .expect("empty flush scheduled blocking work")
            .unwrap();
        assert!(!dir.exists(), "empty flush must not touch disk");
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_and_list_with_writers_keep_selected_versions_coherent() {
    let store = Arc::new(in_memory_store());
    for id in 0..32 {
        store
            .set(
                "s".into(),
                format!("k{id:03}"),
                serde_json::json!({"id":id,"generation":0,"payload":"0".repeat(2048)}),
            )
            .await;
    }
    let gate = Arc::new(tokio::sync::Barrier::new(5));
    let mut tasks = tokio::task::JoinSet::new();
    for reader in 0..4 {
        let store = store.clone();
        let gate = gate.clone();
        tasks.spawn(async move {
            gate.wait().await;
            for _ in 0..20 {
                let values = store.list("s".into()).await;
                assert_eq!(values.len(), 32);
                for (id, value) in values.iter().enumerate() {
                    assert_eq!(value["id"], id);
                    let generation = value["generation"].as_u64().unwrap() as u8;
                    assert!(
                        value["payload"]
                            .as_str()
                            .unwrap()
                            .bytes()
                            .all(|b| b == b'0' + generation)
                    );
                }
                let value = store
                    .get("s".into(), format!("k{reader:03}"))
                    .await
                    .unwrap();
                let generation = value["generation"].as_u64().unwrap() as u8;
                assert!(
                    value["payload"]
                        .as_str()
                        .unwrap()
                        .bytes()
                        .all(|b| b == b'0' + generation)
                );
                tokio::task::yield_now().await;
            }
        });
    }
    gate.wait().await;
    for generation in 1..=3 {
        for id in 0..32 {
            store
                .set(
                    "s".into(),
                    format!("k{id:03}"),
                    serde_json::json!({"id":id,"generation":generation,
                "payload":((b'0'+generation as u8) as char).to_string().repeat(2048)}),
                )
                .await;
            tokio::task::yield_now().await;
        }
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}

// A stale delete must not remove a file for a scope repopulated before the
// delete intent reaches the dirty map. The two halves model the real gap
// between releasing the store lock and recording the dirty operation.
#[tokio::test]
async fn stale_delete_intent_persists_scope_repopulated_before_dirty_mark() {
    let dir = directory();
    let store = manual_store(&dir);
    store
        .set("s".into(), "a".into(), serde_json::json!(1))
        .await;
    store.flush().await.unwrap();

    let stale_delete = {
        let mut guard = store.store.write().await;
        let scope = guard.get_mut("s").unwrap();
        assert!(scope.shift_remove("a").is_some());
        assert!(scope.is_empty());
        DirtyOp::Delete
    };
    store
        .set("s".into(), "b".into(), serde_json::json!(2))
        .await;
    store.dirty.write().await.insert("s".into(), stale_delete);

    store.flush().await.unwrap();
    assert_eq!(
        store.list("s".into()).await,
        vec![StateValue::from(serde_json::json!(2))]
    );
    assert_eq!(read_legacy(&dir, "s")["b"], serde_json::json!(2));
    assert_eq!(
        load_store_from_dir(&dir)["s"]["b"],
        Arc::new(serde_json::json!(2))
    );
    std::fs::remove_dir_all(dir).unwrap();
}

// A stale upsert is only a retry/work marker. The current store decides that
// an empty or absent scope must remove an obsolete on-disk snapshot.
#[tokio::test]
async fn stale_upsert_intent_removes_files_for_empty_and_absent_scopes() {
    let dir = directory();
    let store = manual_store(&dir);
    for scope in ["empty", "absent"] {
        store
            .set(scope.into(), "key".into(), serde_json::json!(scope))
            .await;
    }
    store.flush().await.unwrap();

    {
        let mut guard = store.store.write().await;
        let scope = guard.get_mut("empty").unwrap();
        assert!(scope.shift_remove("key").is_some());
        assert!(scope.is_empty());
        guard.remove("absent");
    }
    {
        let mut dirty = store.dirty.write().await;
        dirty.insert("empty".into(), DirtyOp::Upsert);
        dirty.insert("absent".into(), DirtyOp::Upsert);
    }

    store.flush().await.unwrap();
    assert!(!dir.join(index_file_name("empty")).exists());
    assert!(!dir.join(index_file_name("absent")).exists());
    assert!(!load_store_from_dir(&dir).contains_key("empty"));
    assert!(!load_store_from_dir(&dir).contains_key("absent"));
    assert!(store.list("empty".into()).await.is_empty());
    assert!(store.list("absent".into()).await.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn get_and_list_share_the_same_immutable_record_without_deep_copies() {
    let store = in_memory_store();
    store
        .set(
            "s".into(),
            "k".into(),
            serde_json::json!({"payload":"x".repeat(1024*1024)}),
        )
        .await;
    let get = store.get("s".into(), "k".into()).await.unwrap();
    let list = store.list("s".into()).await;
    {
        let map = store.store.read().await;
        assert!(Arc::ptr_eq(&get.0, &map["s"]["k"]));
        assert!(Arc::ptr_eq(&get.0, &list[0].0));
        assert_eq!(
            get["payload"].as_str().unwrap().as_ptr(),
            map["s"]["k"]["payload"].as_str().unwrap().as_ptr()
        );
    }
    let weak = Arc::downgrade(&get.0);
    store.set("s".into(), "k".into(), Value::Null).await;
    assert_eq!(get["payload"].as_str().unwrap().len(), 1024 * 1024);
    assert_eq!(weak.strong_count(), 2);
    drop(list);
    assert_eq!(weak.strong_count(), 1);
    drop(get);
    assert!(weak.upgrade().is_none());
    assert!(store.get("s".into(), "missing".into()).await.is_none());
}

#[tokio::test]
async fn get_crosses_unchanged_sdk_as_original_json_for_every_value_kind() {
    use iii_sdk::iii::IntoAsyncHandler;
    let store = Arc::new(in_memory_store());
    let handler = {
        let store = store.clone();
        (move |key: String| {
            let store = store.clone();
            async move { Ok::<_, iii_sdk::Error>(store.get("s".into(), key).await) }
        })
        .into_handler()
    };
    let expected = vec![
        Value::Null,
        Value::Bool(true),
        Value::from(18446744073709551615u64),
        serde_json::json!(-4),
        serde_json::json!(1.25),
        serde_json::json!("中文\n\\\""),
        serde_json::json!([1, null, false]),
        serde_json::json!({"a":{"nested":[true]}}),
    ];
    for (i, value) in expected.into_iter().enumerate() {
        store.set("s".into(), i.to_string(), value.clone()).await;
        assert_eq!(
            handler(Value::String(i.to_string()), None).await.unwrap(),
            value
        );
    }
    assert_eq!(
        handler(Value::String("missing".into()), None)
            .await
            .unwrap(),
        Value::Null
    );
    let list = store.list("s".into()).await;
    assert_eq!(
        serde_json::to_value(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        schemars::schema_for!(StateValue).schema,
        schemars::schema_for!(Value).schema
    );
    assert_eq!(
        serde_json::to_value(schemars::schema_for!(Option<StateValue>)).unwrap(),
        serde_json::to_value(schemars::schema_for!(Option<Value>)).unwrap()
    );
    assert_eq!(
        serde_json::to_value(schemars::schema_for!(Vec<StateValue>)).unwrap(),
        serde_json::to_value(schemars::schema_for!(Vec<Value>)).unwrap()
    );
}

#[tokio::test]
async fn shared_removal_preserves_held_snapshot_without_cloning() {
    let store = in_memory_store();
    let value = serde_json::json!({"payload":"x".repeat(1024 * 1024)});
    store.set("s".into(), "k".into(), value.clone()).await;

    let held = store.get("s".into(), "k".into()).await.unwrap();
    let removed = store
        .remove_shared("s".into(), "k".into())
        .await
        .expect("existing key must be removed");

    assert!(Arc::ptr_eq(&held.0, &removed));
    assert_eq!(held.as_ref(), &value);
    assert!(store.get("s".into(), "k".into()).await.is_none());
    assert!(
        store
            .remove_shared("s".into(), "missing".into())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn delete_can_return_its_shared_pre_delete_value_with_unchanged_event_fields() {
    let store = in_memory_store();
    let expected = serde_json::json!({"payload":"x".repeat(65536)});
    store.set("s".into(), "k".into(), expected.clone()).await;
    let before = store.get("s".into(), "k".into()).await;
    drop(store.delete("s".into(), "k".into()).await);
    assert_eq!(serde_json::to_value(&before).unwrap(), expected);
    let event = crate::structs::StateEventData {
        message_type: "state".into(),
        event_type: crate::structs::StateEventType::Deleted,
        scope: "s".into(),
        key: "k".into(),
        old_value: before.as_deref().cloned(),
        new_value: Value::Null,
    };
    assert_eq!(serde_json::to_value(event).unwrap()["old_value"], expected);
}
