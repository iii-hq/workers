// Exercise the crate-private registry without making it part of the public API.
#[path = "../src/cancellation.rs"]
mod cancellation;

use cancellation::CancellationRegistry;
use jev_contract::ErrorCode;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

#[tokio::test]
async fn duplicate_ids_stay_reserved_until_the_guard_is_dropped() {
    let registry = Arc::new(CancellationRegistry::default());
    let guard = registry.start(Some("owner"), Some("request")).unwrap();
    assert!(matches!(
        registry.start(Some("owner"), Some("request")),
        Err(ErrorCode::InvalidRequest)
    ));
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(true));
    assert!(matches!(
        registry.start(Some("owner"), Some("request")),
        Err(ErrorCode::InvalidRequest)
    ));
    drop(guard);
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(false));
    let mut replacement = registry.start(Some("owner"), Some("request")).unwrap();
    assert!(timeout(Duration::from_millis(20), replacement.cancelled())
        .await
        .is_err());
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(true));
    timeout(Duration::from_secs(1), replacement.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancellation_is_isolated_by_exact_caller_and_request_id() {
    let registry = Arc::new(CancellationRegistry::default());
    let mut first = registry.start(Some("first"), Some("shared")).unwrap();
    let mut second = registry.start(Some("second"), Some("shared")).unwrap();
    let mut other = registry.start(Some("first"), Some("other")).unwrap();
    assert_eq!(registry.cancel(Some("stranger"), "shared"), Ok(false));
    assert_eq!(registry.cancel(Some("first"), "missing"), Ok(false));
    assert_eq!(registry.cancel(Some("first"), "shared"), Ok(true));
    timeout(Duration::from_secs(1), first.cancelled())
        .await
        .unwrap();
    for guard in [&mut second, &mut other] {
        assert!(timeout(Duration::from_millis(20), guard.cancelled())
            .await
            .is_err());
    }
    drop(first);
    assert_eq!(registry.cancel(Some("second"), "shared"), Ok(true));
    timeout(Duration::from_secs(1), second.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancellation_before_waiting_is_persistent_across_waits() {
    let registry = Arc::new(CancellationRegistry::default());
    let mut guard = registry.start(Some("owner"), Some("request")).unwrap();
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(true));
    for _ in 0..2 {
        timeout(Duration::from_secs(1), guard.cancelled())
            .await
            .unwrap();
    }
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(true));
}

#[tokio::test]
async fn cancellation_wakes_an_existing_waiter_after_an_abandoned_wait() {
    let registry = Arc::new(CancellationRegistry::default());
    let mut guard = registry.start(Some("owner"), Some("request")).unwrap();
    assert!(timeout(Duration::from_millis(20), guard.cancelled())
        .await
        .is_err());
    let waiting = tokio::spawn(async move { guard.cancelled().await });
    tokio::task::yield_now().await;
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(true));
    timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(false));
}

#[tokio::test]
async fn unidentified_calls_need_no_caller_and_never_cancel() {
    let registry = Arc::new(CancellationRegistry::default());
    for caller in [None, Some(""), Some(" "), Some("owner")] {
        let mut guard = registry.start(caller, None).unwrap();
        assert_eq!(registry.cancel(Some("owner"), "request"), Ok(false));
        assert!(timeout(Duration::from_millis(20), guard.cancelled())
            .await
            .is_err());
    }
}

#[test]
fn identified_calls_and_cancellation_require_a_nonblank_caller() {
    let registry = Arc::new(CancellationRegistry::default());
    for caller in [None, Some(""), Some(" \t\n"), Some("\u{2003}")] {
        assert!(matches!(
            registry.start(caller, Some("request")),
            Err(ErrorCode::InvalidRequest)
        ));
        assert_eq!(
            registry.cancel(caller, "request"),
            Err(ErrorCode::InvalidRequest)
        );
    }
}

#[test]
fn request_ids_are_nonblank_printable_ascii_with_a_128_byte_limit() {
    let registry = Arc::new(CancellationRegistry::default());
    for id in [
        "",
        "   ",
        "a\tb",
        "a\nb",
        "a\rb",
        "a\0b",
        "a\u{7f}b",
        "café",
        &"x".repeat(129),
    ] {
        assert!(
            matches!(
                registry.start(Some("owner"), Some(id)),
                Err(ErrorCode::InvalidRequest)
            ),
            "{id:?}"
        );
        assert_eq!(
            registry.cancel(Some("owner"), id),
            Err(ErrorCode::InvalidRequest),
            "{id:?}"
        );
    }
    for id in ["x", " spaced id ", "!~", &"x".repeat(128)] {
        let guard = registry.start(Some("owner"), Some(id)).unwrap();
        assert_eq!(registry.cancel(Some("owner"), id), Ok(true));
        drop(guard);
    }
}

#[tokio::test]
async fn aborting_a_task_drops_its_guard_and_allows_id_reuse() {
    let registry = Arc::new(CancellationRegistry::default());
    let mut guard = registry.start(Some("owner"), Some("request")).unwrap();
    let task = tokio::spawn(async move { guard.cancelled().await });
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(registry.cancel(Some("owner"), "request"), Ok(false));
    assert!(registry.start(Some("owner"), Some("request")).is_ok());
}
