//! Redis adapter integration: the `WATCH`/`MULTI` compare-and-set path that
//! unit tests cannot reach without a server.
//!
//! Connect-or-skip like the engine e2e suite: unreachable Redis skips the
//! test (CI and redis-less boxes stay green); set `III_REDIS_REQUIRE` to any
//! value to fail loudly instead. Point at a server with `III_REDIS_URL`
//! (default `redis://localhost:6379`).

use std::sync::Arc;

use iii_state::adapters::{CompareAndSetOutcome, StateAdapter, build_adapter};
use iii_state::config::StateConfig;
use serde_json::json;
use uuid::Uuid;

async fn connect() -> Option<Arc<dyn StateAdapter>> {
    let url =
        std::env::var("III_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let config: StateConfig = serde_json::from_value(json!({
        "adapter": {"name": "redis", "config": {"redis_url": url}},
    }))
    .expect("redis state config");
    match build_adapter(&config).await {
        Ok(adapter) => Some(adapter),
        Err(error) => {
            if std::env::var("III_REDIS_REQUIRE").is_ok() {
                panic!("III_REDIS_REQUIRE is set but Redis is unreachable at {url}: {error}");
            }
            eprintln!(
                "[skip] no Redis reachable at {url} — skipping (set III_REDIS_REQUIRE=1 to fail \
                 instead)"
            );
            None
        }
    }
}

#[tokio::test]
async fn redis_cas_swaps_only_on_match() {
    let Some(adapter) = connect().await else {
        return;
    };
    // Redis outlives the test process — a unique scope isolates reruns.
    let scope = format!("e2e-redis-cas-{}", Uuid::new_v4());

    // Set-if-absent claims the key and reports no previous value.
    let claimed = adapter
        .compare_and_set(&scope, "slot", None, json!({"owner": "a", "n": 1}))
        .await
        .expect("set-if-absent");
    assert_eq!(claimed, CompareAndSetOutcome::Swapped { old_value: None });

    // A second set-if-absent loses and returns the current value.
    let lost = adapter
        .compare_and_set(&scope, "slot", None, json!({"owner": "b"}))
        .await
        .expect("competing set-if-absent");
    assert_eq!(
        lost,
        CompareAndSetOutcome::NotSwapped {
            current: json!({"owner": "a", "n": 1})
        }
    );

    // Equality is parsed-JSON equality: object key order must not matter.
    let reordered = serde_json::from_str(r#"{"n":1,"owner":"a"}"#).unwrap();
    let swapped = adapter
        .compare_and_set(&scope, "slot", Some(&reordered), json!({"owner": "b"}))
        .await
        .expect("reordered-expectation CAS");
    assert!(
        matches!(swapped, CompareAndSetOutcome::Swapped { old_value: Some(v) } if v == json!({"owner": "a", "n": 1}))
    );

    // A stored null counts as absent — in both directions.
    adapter
        .set(&scope, "nulled", serde_json::Value::Null)
        .await
        .expect("seed a stored null");
    let over_null = adapter
        .compare_and_set(
            &scope,
            "nulled",
            Some(&serde_json::Value::Null),
            json!("filled"),
        )
        .await
        .expect("CAS with a null expectation");
    assert!(matches!(over_null, CompareAndSetOutcome::Swapped { .. }));

    // The redis adapter refuses barriers rather than faking atomicity.
    let cfg = iii_state::barrier::BarrierConfig {
        id: "join".into(),
        expect: iii_state::barrier::Expect::Count(2),
        key_from: None,
        carry: None,
    };
    let refusal = adapter
        .barrier_arrive(&scope, "join", &cfg, &json!({"key": "a"}))
        .await
        .expect_err("redis barriers must refuse, not race");
    assert!(refusal.to_string().contains("kv adapter"), "{refusal}");

    for key in ["slot", "nulled"] {
        adapter.delete(&scope, key).await.expect("cleanup");
    }
}

#[tokio::test]
async fn redis_pagination_keeps_atomic_hash_snapshot_versions_and_rejects_corruption() {
    use iii_state::pagination::{EntryPages, PageError};
    use iii_state::structs::StateListEntriesInput;
    use serde_json::Value;
    let Some(adapter) = connect().await else {
        return;
    };
    let scope = format!("pagination-{}", Uuid::new_v4());
    for (key, value) in [
        ("a", Value::Null),
        ("b", json!({"version":1})),
        ("c", json!("old")),
    ] {
        adapter.set(&scope, key, value).await.unwrap();
    }
    let request = |cursor| StateListEntriesInput {
        scope: scope.clone(),
        cursor,
        limit: Some(1),
        max_bytes: Some(1_000_000),
        caller_worker_id: Some("redis-test".into()),
    };
    let pages = EntryPages::default();
    let first = pages.list(&adapter, "public", request(None)).await.unwrap();
    adapter
        .set(&scope, "b", json!({"version":2}))
        .await
        .unwrap();
    adapter.delete(&scope, "c").await.unwrap();
    adapter.set(&scope, "new", json!("inserted")).await.unwrap();
    let mut entries = first.entries;
    let mut cursor = first.next_cursor;
    while let Some(token) = cursor {
        let page = pages
            .list(&adapter, "public", request(Some(token)))
            .await
            .unwrap();
        assert_eq!(page.total, 3);
        entries.extend(page.entries);
        cursor = page.next_cursor;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(entries.len(), 3);
    assert!(entries[0].1.is_null());
    assert_eq!(entries[1].1.as_ref(), &json!({"version":1}));
    assert_eq!(entries[2].1.as_ref(), &json!("old"));
    let url = std::env::var("III_REDIS_URL").unwrap();
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("HSET")
        .arg(format!("state:{scope}"))
        .arg("corrupt")
        .arg("not-json")
        .query_async::<usize>(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        pages
            .list(&adapter, "public", request(None))
            .await
            .unwrap_err(),
        PageError::Adapter
    );
    redis::cmd("HDEL")
        .arg(format!("state:{scope}"))
        .arg("corrupt")
        .query_async::<usize>(&mut conn)
        .await
        .unwrap();
    adapter
        .set(&scope, "huge", json!("x".repeat(8_000_000)))
        .await
        .unwrap();
    assert_eq!(
        pages
            .list(&adapter, "public", request(None))
            .await
            .unwrap_err(),
        PageError::RowTooLarge
    );
    redis::cmd("DEL")
        .arg(format!("state:{scope}"))
        .query_async::<usize>(&mut conn)
        .await
        .unwrap();
    println!(
        "Redis atomic HGETALL snapshot: null, update/delete/insert between pages, corrupt capture and huge-row refusal verified"
    );
}

#[tokio::test]
async fn redis_private_non_null_history_preserves_atomic_versions_keys_and_default_caps() {
    use iii_state::pagination::{EntryPages, PageError};
    use iii_state::structs::{StateListEntriesInput, StatePrivateListEntriesInput};
    use serde_json::Value;
    let Some(adapter) = connect().await else {
        return;
    };
    let scope = format!("b1-history-{}", Uuid::new_v4());
    let active_scope = format!("b1-active-{}", Uuid::new_v4());
    let client = redis::Client::open(std::env::var("III_REDIS_URL").unwrap()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let hash = format!("state:{scope}");
    let mut seed = redis::cmd("HSET");
    seed.arg(&hash);
    for i in 0..100_001 {
        seed.arg(format!("retired-{i:06}")).arg("null");
    }
    assert_eq!(seed.query_async::<usize>(&mut conn).await.unwrap(), 100_001);
    let original = [
        (
            "target",
            json!({"session_id":"target","function_id":"slow::target"}),
        ),
        ("ambiguous", json!({"function_id":"no-owner"})),
        ("foreign", json!({"session_id":"foreign"})),
    ];
    for (key, value) in &original {
        adapter.set(&scope, key, value.clone()).await.unwrap();
    }
    let request = |cursor, non_null_only| StatePrivateListEntriesInput {
        page: StateListEntriesInput {
            scope: scope.clone(),
            cursor,
            limit: Some(1),
            max_bytes: Some(1_000_000),
            caller_worker_id: Some("redis-b1".into()),
        },
        non_null_only,
    };
    let pages = EntryPages::default();
    assert_eq!(
        pages
            .list(&adapter, "state::list_entries", request(None, false).page)
            .await
            .unwrap_err(),
        PageError::SnapshotCapacity
    );
    assert_eq!(
        pages
            .list_private(&adapter, "harness", request(None, false))
            .await
            .unwrap_err(),
        PageError::SnapshotCapacity
    );
    let first = pages
        .list_private(&adapter, "harness", request(None, true))
        .await
        .unwrap();
    assert_eq!(first.total, 3);
    let cursor = first.next_cursor.clone();
    assert_eq!(
        pages
            .list_private(&adapter, "harness", request(cursor.clone(), false))
            .await
            .unwrap_err(),
        PageError::InvalidCursor
    );
    let writer = adapter.clone();
    let writer_scope = scope.clone();
    tokio::spawn(async move {
        writer
            .set(&writer_scope, "target", Value::Null)
            .await
            .unwrap();
        writer
            .set(
                &writer_scope,
                "ambiguous",
                json!({"session_id":"replacement"}),
            )
            .await
            .unwrap();
        writer
            .set(&writer_scope, "retired-000000", json!({"session_id":"new"}))
            .await
            .unwrap();
    })
    .await
    .unwrap();
    let mut seen = first.entries;
    let mut cursor = cursor;
    while let Some(token) = cursor {
        let page = pages
            .list_private(&adapter, "harness", request(Some(token), true))
            .await
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.offset, seen.len());
        assert!(serde_json::to_vec(&page).unwrap().len() <= 1_000_000);
        seen.extend(page.entries);
        cursor = page.next_cursor;
    }
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected = original.to_vec();
    expected.sort_by(|a, b| a.0.cmp(b.0));
    assert_eq!(
        seen.iter()
            .map(|(k, v)| (k.as_str(), v.as_ref()))
            .collect::<Vec<_>>(),
        expected.iter().map(|(k, v)| (*k, v)).collect::<Vec<_>>()
    );
    assert_eq!(
        redis::cmd("HLEN")
            .arg(&hash)
            .query_async::<usize>(&mut conn)
            .await
            .unwrap(),
        100_004
    );
    assert_eq!(
        redis::cmd("HGET")
            .arg(&hash)
            .arg("retired-000001")
            .query_async::<String>(&mut conn)
            .await
            .unwrap(),
        "null"
    );
    // Corrupt JSON is not equivalent to null and cannot be silently filtered.
    redis::cmd("HSET")
        .arg(&hash)
        .arg("corrupt")
        .arg("not-json")
        .query_async::<usize>(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        pages
            .list_private(&adapter, "harness", request(None, true))
            .await
            .unwrap_err(),
        PageError::Adapter
    );
    redis::cmd("HDEL")
        .arg(&hash)
        .arg("corrupt")
        .query_async::<usize>(&mut conn)
        .await
        .unwrap();
    adapter
        .set(&scope, "huge", json!("x".repeat(8_000_000)))
        .await
        .unwrap();
    assert_eq!(
        pages
            .list_private(&adapter, "harness", request(None, true))
            .await
            .unwrap_err(),
        PageError::RowTooLarge
    );
    let mut seed = redis::cmd("HSET");
    seed.arg(format!("state:{active_scope}"));
    for i in 0..100_001 {
        seed.arg(format!("active-{i:06}")).arg("false");
    }
    seed.query_async::<usize>(&mut conn).await.unwrap();
    let mut active = request(None, true);
    active.page.scope = active_scope.clone();
    assert_eq!(
        pages
            .list_private(&adapter, "harness", active)
            .await
            .unwrap_err(),
        PageError::SnapshotCapacity
    );
    // Only test-owned scopes are removed after assertions; application reads
    // never mutate or compact their null history.
    redis::cmd("DEL")
        .arg(&hash)
        .arg(format!("state:{active_scope}"))
        .query_async::<usize>(&mut conn)
        .await
        .unwrap();
    println!(
        "B1 Redis: 100001 retired null keys + 3 non-null witnesses; filtered atomic snapshot, mode binding, physical keys, public/default/active caps, corruption/huge refusal verified"
    );
}
