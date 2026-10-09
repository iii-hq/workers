use super::*;
use crate::adapters::KvStoreAdapter;
use serde_json::{Value, json};
fn adapter() -> Arc<dyn StateAdapter> {
    Arc::new(KvStoreAdapter::new(Some(
        json!({"store_method":"in_memory"}),
    )))
}
fn input(cursor: Option<String>) -> StateListEntriesInput {
    StateListEntriesInput {
        scope: "s".into(),
        cursor,
        limit: Some(1),
        max_bytes: Some(1_000_000),
        caller_worker_id: Some("caller".into()),
    }
}
#[tokio::test]
async fn empty_single_null_and_mutating_multi_page_snapshot() {
    let pages = EntryPages::default();
    let a = adapter();
    let empty = pages.list(&a, "public", input(None)).await.unwrap();
    assert!(empty.done && empty.entries.is_empty() && empty.next_cursor.is_none());
    a.set("s", "a", Value::Null).await.unwrap();
    let single = pages.list(&a, "public", input(None)).await.unwrap();
    assert!(single.done && single.entries[0].1.is_null());
    a.set("s", "b", json!({"old":true})).await.unwrap();
    a.set("s", "c", json!("deleted later")).await.unwrap();
    let first = pages.list(&a, "public", input(None)).await.unwrap();
    let token = first.next_cursor.unwrap();
    a.set("s", "b", json!({"new":true})).await.unwrap();
    a.delete("s", "c").await.unwrap();
    a.set("s", "inserted", json!(42)).await.unwrap();
    let second = pages
        .list(&a, "public", input(Some(token.clone())))
        .await
        .unwrap();
    assert_eq!(second.entries[0].1.as_ref(), &json!({"old":true}));
    assert_eq!(
        pages
            .list(&a, "public", input(Some(token)))
            .await
            .unwrap_err(),
        PageError::InvalidCursor
    );
    let third = pages
        .list(&a, "public", input(second.next_cursor))
        .await
        .unwrap();
    assert_eq!(third.entries[0].0, "c");
    assert_eq!(third.total, 3);
    assert_eq!(third.offset, 2);
    assert!(third.done && third.next_cursor.is_none());
    assert!(pages.cache.lock().await.snapshots.is_empty());
}
#[tokio::test]
async fn cursors_are_scope_namespace_caller_limit_bound_and_expire() {
    let pages = EntryPages::default();
    let a = adapter();
    for key in ["a", "b"] {
        a.set("s", key, json!(key)).await.unwrap();
    }
    let token = pages
        .list(&a, "public", input(None))
        .await
        .unwrap()
        .next_cursor
        .unwrap();
    for case in ["scope", "caller", "limit", "bytes", "tamper", "namespace"] {
        let mut request = input(Some(token.clone()));
        match case {
            "scope" => request.scope = "other".into(),
            "caller" => request.caller_worker_id = Some("other".into()),
            "limit" => request.limit = Some(2),
            "bytes" => request.max_bytes = Some(999_999),
            "tamper" => request.cursor = Some(Uuid::new_v4().to_string()),
            _ => {}
        }
        let namespace = if case == "namespace" {
            "private"
        } else {
            "public"
        };
        assert_eq!(
            pages.list(&a, namespace, request).await.unwrap_err(),
            PageError::InvalidCursor,
            "{case}"
        );
    }
    pages
        .cache
        .lock()
        .await
        .snapshots
        .get_mut(&token)
        .unwrap()
        .expires = Instant::now();
    assert_eq!(
        pages
            .list(&a, "public", input(Some(token)))
            .await
            .unwrap_err(),
        PageError::InvalidCursor
    );
    let mut missing = input(None);
    missing.caller_worker_id = None;
    assert_eq!(
        pages.list(&a, "public", missing).await.unwrap_err(),
        PageError::UnknownCaller
    );
    assert_eq!(
        EntryPages::default()
            .list(&a, "public", input(Some(Uuid::new_v4().to_string())))
            .await
            .unwrap_err(),
        PageError::InvalidCursor
    );
}
#[tokio::test]
async fn invalid_limits_and_snapshot_count_exhaustion_do_not_evict_live_cursor() {
    let pages = EntryPages::default();
    let a = adapter();
    for key in ["a", "b"] {
        a.set("s", key, Value::Null).await.unwrap();
    }
    for (limit, bytes) in [(0, 256), (1001, 256), (1, 255), (1, 8_000_001)] {
        let mut request = input(None);
        request.limit = Some(limit);
        request.max_bytes = Some(bytes);
        assert_eq!(
            pages.list(&a, "public", request).await.unwrap_err(),
            PageError::InvalidLimits
        );
    }
    let mut token = None;
    for _ in 0..MAX_SNAPSHOTS {
        token = pages
            .list(&a, "public", input(None))
            .await
            .unwrap()
            .next_cursor;
    }
    assert_eq!(
        pages.list(&a, "public", input(None)).await.unwrap_err(),
        PageError::SnapshotCapacity
    );
    assert!(pages.list(&a, "public", input(token)).await.unwrap().done);
}
#[tokio::test]
async fn exact_encoded_boundary_includes_long_escaped_keys_unicode_and_cursor() {
    let pages = EntryPages::default();
    let a = adapter();
    let key = "中文\n\"\\\u{0001}".repeat(80);
    a.set("s", &key, json!("é\n\"\\\u{0000}".repeat(30)))
        .await
        .unwrap();
    a.set("s", "later", Value::Null).await.unwrap();
    let first = pages.list(&a, "public", input(None)).await.unwrap();
    let bytes = serde_json::to_vec(&first).unwrap().len();
    let mut request = input(None);
    request.max_bytes = Some(bytes);
    let exact = pages.list(&a, "public", request.clone()).await.unwrap();
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), bytes);
    assert!(!exact.done);
    request.max_bytes = Some(bytes - 1);
    assert_eq!(
        pages.list(&a, "public", request).await.unwrap_err(),
        PageError::RowTooLarge
    );
    println!("escaped-key nonterminal response exact boundary={bytes} bytes");
}
#[tokio::test]
async fn oversized_first_or_later_row_is_explicit_never_a_terminal_skip() {
    let pages = EntryPages::default();
    let a = adapter();
    a.set("s", "small", Value::Null).await.unwrap();
    a.set("s", "large", json!("x".repeat(500))).await.unwrap();
    let mut request = input(None);
    request.max_bytes = Some(256);
    let first = pages.list(&a, "public", request.clone()).await.unwrap();
    assert!(!first.done);
    request.cursor = first.next_cursor.clone();
    assert_eq!(
        pages.list(&a, "public", request).await.unwrap_err(),
        PageError::RowTooLarge
    );
    assert!(
        pages
            .cache
            .lock()
            .await
            .snapshots
            .contains_key(first.next_cursor.as_ref().unwrap())
    );
    let huge = adapter();
    huge.set("s", "secret-key", json!("x".repeat(MAX_PAGE_BYTES)))
        .await
        .unwrap();
    let error = pages.list(&huge, "public", input(None)).await.unwrap_err();
    assert_eq!(error, PageError::RowTooLarge);
    assert!(error.to_string().len() < 256);
    assert!(!error.to_string().contains("secret-key"));
}
#[tokio::test]
async fn retained_byte_limits_fail_without_dropping_live_snapshots() {
    let pages = EntryPages::default();
    let a = adapter();
    for i in 0..5 {
        a.set("s", &i.to_string(), json!("x".repeat(7_000_000)))
            .await
            .unwrap();
    }
    assert_eq!(
        pages.list(&a, "public", input(None)).await.unwrap_err(),
        PageError::SnapshotCapacity
    );
    a.delete("s", "4").await.unwrap();
    let mut request = input(None);
    request.max_bytes = Some(MAX_PAGE_BYTES);
    let first = pages.list(&a, "public", request.clone()).await.unwrap();
    let second = pages.list(&a, "public", request.clone()).await.unwrap();
    assert_eq!(
        pages.list(&a, "public", request.clone()).await.unwrap_err(),
        PageError::SnapshotCapacity
    );
    request.cursor = first.next_cursor;
    assert!(pages.list(&a, "public", request).await.is_ok());
    assert!(
        pages
            .cache
            .lock()
            .await
            .snapshots
            .contains_key(second.next_cursor.as_ref().unwrap())
    );
}

#[tokio::test]
async fn byte_limit_splits_before_count_limit_and_terminal_boundary_is_exact() {
    let pages = EntryPages::default();
    let a = adapter();
    for i in 0..7 {
        a.set("s", &format!("key-{i}"), json!("中文\"\n".repeat(30)))
            .await
            .unwrap();
    }
    let mut request = input(None);
    request.limit = Some(1000);
    request.max_bytes = Some(600);
    let mut offset = 0;
    let mut seen = Vec::new();
    let mut count = 0;
    loop {
        let page = pages.list(&a, "public", request.clone()).await.unwrap();
        assert_eq!(page.offset, offset);
        assert_eq!(page.total, 7);
        assert!(serde_json::to_vec(&page).unwrap().len() <= 600);
        assert!(!page.entries.is_empty());
        offset += page.entries.len();
        seen.extend(page.entries);
        count += 1;
        request.cursor = page.next_cursor;
        if page.done {
            assert!(request.cursor.is_none());
            break;
        }
    }
    assert_eq!(seen.len(), 7);
    assert!(count > 1);
    let one = adapter();
    one.set("s", "terminal\n\"", json!("中文".repeat(100)))
        .await
        .unwrap();
    let terminal = pages.list(&one, "public", input(None)).await.unwrap();
    let bytes = serde_json::to_vec(&terminal).unwrap().len();
    let mut request = input(None);
    request.max_bytes = Some(bytes);
    assert!(
        pages
            .list(&one, "public", request.clone())
            .await
            .unwrap()
            .done
    );
    request.max_bytes = Some(bytes - 1);
    assert_eq!(
        pages.list(&one, "public", request).await.unwrap_err(),
        PageError::RowTooLarge
    );
    println!("byte-limited traversal pages={count}; terminal boundary={bytes} bytes");
}
#[tokio::test]
async fn row_retention_bound_is_explicit_and_value_versions_are_shared() {
    let pages = EntryPages::default();
    let a = adapter();
    a.set("s", "a", json!({"nested":["old"]})).await.unwrap();
    a.set("s", "b", Value::Null).await.unwrap();
    let before = a.get("s", "a").await.unwrap().unwrap();
    let page = pages.list(&a, "public", input(None)).await.unwrap();
    assert!(Arc::ptr_eq(&before.0, &page.entries[0].1.0));
    let keys = a.list_keys("s").await.unwrap();
    assert_eq!(keys, ["a", "b"]);
    let many = adapter();
    for i in 0..=MAX_SNAPSHOT_ENTRIES {
        many.set("s", &i.to_string(), Value::Null).await.unwrap();
    }
    assert_eq!(
        pages.list(&many, "public", input(None)).await.unwrap_err(),
        PageError::SnapshotCapacity
    );
    assert!(
        pages
            .cache
            .lock()
            .await
            .snapshots
            .contains_key(page.next_cursor.as_ref().unwrap())
    );
}

#[tokio::test]
async fn full_terminal_tail_can_fit_even_when_its_first_row_with_cursor_cannot() {
    let pages = EntryPages::default();
    let a = adapter();
    a.set("s", "a", json!("x".repeat(150))).await.unwrap();
    a.set("s", "b", Value::Null).await.unwrap();
    let mut request = input(None);
    request.limit = Some(2);
    request.max_bytes = Some(256);
    let page = pages.list(&a, "public", request).await.unwrap();
    assert!(page.done && page.entries.len() == 2);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 256);
    assert!(pages.cache.lock().await.snapshots.is_empty());
}

#[tokio::test]
async fn continuation_does_not_extend_snapshot_ttl_and_terminal_releases_byte_charge() {
    let pages = EntryPages::default();
    let a = adapter();
    for key in ["a", "b", "c"] {
        a.set("s", key, Value::Null).await.unwrap();
    }
    let first = pages.list(&a, "public", input(None)).await.unwrap();
    let expires = pages.cache.lock().await.snapshots[first.next_cursor.as_ref().unwrap()].expires;
    let second = pages
        .list(&a, "public", input(first.next_cursor))
        .await
        .unwrap();
    let cache = pages.cache.lock().await;
    assert_eq!(
        cache.snapshots[second.next_cursor.as_ref().unwrap()].expires,
        expires
    );
    assert!(cache.bytes > 0);
    drop(cache);
    assert!(
        pages
            .list(&a, "public", input(second.next_cursor))
            .await
            .unwrap()
            .done
    );
    let cache = pages.cache.lock().await;
    assert!(cache.snapshots.is_empty());
    assert_eq!(cache.bytes, 0);
}

#[tokio::test]
async fn oversized_snapshot_identity_is_rejected_before_retention() {
    let pages = EntryPages::default();
    let a = adapter();
    let mut request = input(None);
    request.scope = "s".repeat(1025);
    assert_eq!(
        pages.list(&a, "public", request).await.unwrap_err(),
        PageError::InvalidLimits
    );
    assert_eq!(
        pages
            .list(&a, &"n".repeat(1025), input(None))
            .await
            .unwrap_err(),
        PageError::InvalidLimits
    );
    let mut request = input(None);
    request.caller_worker_id = Some("c".repeat(257));
    assert_eq!(
        pages.list(&a, "public", request).await.unwrap_err(),
        PageError::UnknownCaller
    );
    assert!(pages.cache.lock().await.snapshots.is_empty());
}

fn private_input(cursor: Option<String>, non_null_only: bool) -> StatePrivateListEntriesInput {
    StatePrivateListEntriesInput {
        page: input(cursor),
        non_null_only,
    }
}

#[tokio::test]
async fn private_non_null_snapshot_filters_history_before_caps_but_keeps_ambiguous_and_foreign_rows()
 {
    let pages = EntryPages::default();
    let a = adapter();
    for i in 0..=MAX_SNAPSHOT_ENTRIES {
        let key = format!("retired-{i}");
        let witness = json!({"session_id":"retired","function_id":"historical::call"});
        assert!(matches!(
            a.compare_and_set("s", &key, None, witness.clone())
                .await
                .unwrap(),
            crate::adapters::CompareAndSetOutcome::Swapped { .. }
        ));
        assert!(matches!(
            a.compare_and_set("s", &key, Some(&witness), Value::Null)
                .await
                .unwrap(),
            crate::adapters::CompareAndSetOutcome::Swapped { .. }
        ));
    }
    let empty = pages
        .list_private(&a, "harness", private_input(None, true))
        .await
        .unwrap();
    assert!(empty.done && empty.next_cursor.is_none() && empty.entries.is_empty());
    assert_eq!(empty.total, 0);
    assert_eq!(a.list_keys("s").await.unwrap().len(), 100_001);
    for (key, value) in [
        (
            "target",
            json!({"session_id":"target","function_id":"slow::target"}),
        ),
        ("ambiguous", json!({"function_id":"no-owner"})),
        ("foreign", json!({"session_id":"foreign"})),
    ] {
        a.set("s", key, value).await.unwrap();
    }
    assert_eq!(
        pages
            .list(&a, "state::list_entries", input(None))
            .await
            .unwrap_err(),
        PageError::SnapshotCapacity
    );
    assert_eq!(
        pages
            .list_private(&a, "harness", private_input(None, false))
            .await
            .unwrap_err(),
        PageError::SnapshotCapacity
    );
    let first = pages
        .list_private(&a, "harness", private_input(None, true))
        .await
        .unwrap();
    assert_eq!(first.total, 3);
    {
        let cache = pages.cache.lock().await;
        let snapshot = &cache.snapshots[first.next_cursor.as_ref().unwrap()];
        assert_eq!(
            snapshot.entries.capacity(),
            3,
            "whole-history Vec capacity must not be retained"
        );
        assert!(snapshot.bytes < 1_000);
    }
    let token = first.next_cursor.clone();
    assert_eq!(
        pages
            .list_private(&a, "harness", private_input(token.clone(), false))
            .await
            .unwrap_err(),
        PageError::InvalidCursor
    );
    // Intervening writes change both filter membership directions, not the snapshot.
    a.set("s", "ambiguous", Value::Null).await.unwrap();
    a.delete("s", "foreign").await.unwrap();
    a.set("s", "retired-0", json!({"session_id":"new"}))
        .await
        .unwrap();
    a.set("s", "inserted", json!({"session_id":"new"}))
        .await
        .unwrap();
    let mut seen = first.entries;
    let mut cursor = token;
    while let Some(token) = cursor {
        let page = pages
            .list_private(&a, "harness", private_input(Some(token), true))
            .await
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.offset, seen.len());
        assert!(serde_json::to_vec(&page).unwrap().len() <= 1_000_000);
        seen.extend(page.entries);
        cursor = page.next_cursor;
    }
    assert_eq!(
        seen.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
        ["target", "ambiguous", "foreign"]
    );
    assert_eq!(seen[1].1.as_ref(), &json!({"function_id":"no-owner"}));
    assert_eq!(seen[2].1.as_ref(), &json!({"session_id":"foreign"}));
    assert!(a.get("s", "retired-1").await.unwrap().unwrap().is_null());
    println!(
        "B1 KV: 100001 retired null keys filtered before caps; target/ambiguous/foreign captured and mode switch rejected"
    );
}

#[tokio::test]
async fn non_null_filter_is_exact_and_does_not_relax_active_retention_or_row_budgets() {
    let pages = EntryPages::default();
    let a = adapter();
    for (key, value) in [
        ("null", Value::Null),
        ("false", json!(false)),
        ("zero", json!(0)),
        ("empty", json!("")),
        ("array", json!([])),
        ("object", json!({})),
        ("string-null", json!("null")),
    ] {
        a.set("s", key, value).await.unwrap();
    }
    let mut request = private_input(None, true);
    request.page.limit = Some(100);
    let page = pages.list_private(&a, "harness", request).await.unwrap();
    assert_eq!(page.total, 6);
    assert!(page.done);
    assert!(page.entries.iter().all(|(_, v)| !v.is_null()));
    for mode in [false, true] {
        let mut request = private_input(None, mode);
        request.page.limit = Some(100);
        assert_eq!(
            pages
                .list_private(&a, "harness", request)
                .await
                .unwrap()
                .total,
            if mode { 6 } else { 7 }
        );
    }
    for i in 0..=MAX_SNAPSHOT_ENTRIES {
        a.set("active", &i.to_string(), json!(false)).await.unwrap();
    }
    let mut request = private_input(None, true);
    request.page.scope = "active".into();
    assert_eq!(
        pages
            .list_private(&a, "harness", request)
            .await
            .unwrap_err(),
        PageError::SnapshotCapacity
    );
    a.set("huge", "null", Value::Null).await.unwrap();
    a.set("huge", "live", json!("x".repeat(MAX_PAGE_BYTES)))
        .await
        .unwrap();
    let mut request = private_input(None, true);
    request.page.scope = "huge".into();
    request.page.max_bytes = Some(MAX_PAGE_BYTES);
    assert_eq!(
        pages
            .list_private(&a, "harness", request)
            .await
            .unwrap_err(),
        PageError::RowTooLarge
    );
    // The reverse mode switch also must not consume a default-mode cursor.
    let default = pages
        .list_private(&a, "harness", private_input(None, false))
        .await
        .unwrap();
    assert_eq!(
        pages
            .list_private(
                &a,
                "harness",
                private_input(default.next_cursor.clone(), true)
            )
            .await
            .unwrap_err(),
        PageError::InvalidCursor
    );
    assert_eq!(
        pages
            .list_private(&a, "harness", private_input(default.next_cursor, false))
            .await
            .unwrap()
            .offset,
        1
    );
}
