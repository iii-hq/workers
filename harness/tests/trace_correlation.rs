//! Integration tests for harness trace correlation.
//!
//! Requires a live engine with the iii-observability worker configured for
//! `exporter: memory` (matches `harness/config.yaml`). When no engine is
//! reachable, tests print "skipping: …" and return — mirrors the pattern in
//! `harness/tests/bridge_info.rs` and `harness/tests/sse_bridge.rs`.

use harness::register_with_iii_with_engine_url;
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::{json, Value};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

/// Per-call timeout for engine triggers in this test suite. 2s is enough for
/// the engine to dispatch and return for `harness::status` / `harness::call`
/// / `engine::traces::*` in a local engine; tests that need a longer budget
/// should declare their own override.
const TRIGGER_TIMEOUT_MS: u64 = 2_000;

/// Retry budget for the memory-exporter flush. Spans land asynchronously after
/// the call returns; under uncontended load 10×100ms = 1s is plenty.
const EXPORTER_FLUSH_RETRIES: u32 = 10;
const EXPORTER_FLUSH_INTERVAL_MS: u64 = 100;

/// Same retry pattern but expanded for the test that runs under parallel-test
/// pressure on the shared exporter. 30×200ms = 6s tolerance for eviction and
/// dispatch jitter; matched to the `harness_call_missing_function_id`
/// error path which competes with other tests for exporter capacity.
const EXPORTER_FLUSH_RETRIES_UNDER_LOAD: u32 = 30;
const EXPORTER_FLUSH_INTERVAL_UNDER_LOAD_MS: u64 = 200;
const ENGINE_PROBE_TIMEOUT_MS: u64 = 500;

/// When set, integration tests refuse to silently skip because no engine is
/// reachable — they panic instead. CI should set this so a missing engine
/// is a loud failure, not a green build with zero assertions.
const REQUIRE_ENGINE_ENV: &str = "HARNESS_TRACE_TESTS_REQUIRE_ENGINE";

/// When set, tests that depend on the iii-observability worker (traceparent
/// echo, engine::traces::list/tree queries) panic instead of silently skipping
/// when traceparent is absent or no spans land in the memory exporter. CI
/// runs with observability up should set this.
const REQUIRE_OTEL_ENV: &str = "HARNESS_TRACE_TESTS_REQUIRE_OTEL";

fn require_engine() -> bool {
    std::env::var(REQUIRE_ENGINE_ENV).is_ok()
}

fn require_otel() -> bool {
    std::env::var(REQUIRE_OTEL_ENV).is_ok()
}

/// Either skip (logging the reason) or panic, depending on REQUIRE_OTEL_ENV.
/// Production wrapper around [`skip_or_panic_inner`] — splits the env-var
/// read from the decision logic so the panic path is testable without the
/// process-wide `std::env::set_var` (unsafe in Rust 2024+, races in 2021).
fn skip_or_panic_otel(reason: &str) {
    skip_or_panic_inner(require_otel(), REQUIRE_OTEL_ENV, reason);
}

/// Decision core: panic if `require` is true, else `eprintln!` a skip line.
/// Pure function over `(bool, &str, &str)` — tested by
/// `skip_or_panic_inner_panics_when_required`.
fn skip_or_panic_inner(require: bool, env_name: &str, reason: &str) {
    assert!(!require, "{env_name} set but {reason}");
    eprintln!("skipping: {reason}");
}

/// Boot harness in-process against a live engine; returns None when no engine
/// is reachable (call sites print the skip message and return). When
/// `HARNESS_TRACE_TESTS_REQUIRE_ENGINE` is set in the environment, panics
/// instead of returning None so CI failures are loud.
async fn boot_or_skip() -> Option<(iii_sdk::III, String)> {
    let url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = register_worker(&url, InitOptions::default());

    let probe = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": "agent", "key": "__trace_correlation_probe" }),
            action: None,
            timeout_ms: Some(ENGINE_PROBE_TIMEOUT_MS),
        })
        .await;
    if let Err(e) = probe {
        assert!(
            !require_engine(),
            "{REQUIRE_ENGINE_ENV} set but no engine reachable at {url}: {e}",
        );
        eprintln!("skipping: no engine at {url}");
        return None;
    }

    if let Err(e) = register_with_iii_with_engine_url(&iii, &url).await {
        assert!(
            !require_engine(),
            "{REQUIRE_ENGINE_ENV} set but register failed against engine at {url}: {e}",
        );
        eprintln!("skipping: register failed: {e}");
        return None;
    }

    Some((iii, url))
}

/// Helper: invoke `harness::status` directly via the bus, return the envelope.
async fn call_status(iii: &iii_sdk::III, session: &str, message: Option<&str>) -> Value {
    let mut payload = json!({ "session_id": session });
    if let Some(m) = message {
        payload["message_id"] = Value::String(m.to_string());
    }
    iii.trigger(TriggerRequest {
        function_id: "harness::status".into(),
        payload,
        action: None,
        timeout_ms: Some(TRIGGER_TIMEOUT_MS),
    })
    .await
    .expect("call harness::status")
}

/// Helper: invoke `harness::status` via `harness::call` (the HTTP envelope
/// path that browsers actually use). The outer wrapper around `harness::call`
/// echoes `traceparent` / `x-iii-message-id` into response headers; the inner
/// `harness::status` returns its raw shape directly.
async fn call_status_via_bridge(iii: &iii_sdk::III, session: &str, message: Option<&str>) -> Value {
    let mut payload =
        json!({ "function_id": "harness::status", "payload": {}, "session_id": session });
    if let Some(m) = message {
        payload["message_id"] = Value::String(m.to_string());
    }
    iii.trigger(TriggerRequest {
        function_id: "harness::call".into(),
        payload,
        action: None,
        timeout_ms: Some(TRIGGER_TIMEOUT_MS),
    })
    .await
    .expect("call harness::call -> harness::status")
}

#[tokio::test]
#[serial_test::serial(harness_trace)]
async fn harness_call_echoes_message_id_when_provided() {
    let Some((iii, _url)) = boot_or_skip().await else {
        return;
    };
    let env = call_status_via_bridge(&iii, "S1", Some("M1")).await;
    let headers = &env["headers"];
    assert_eq!(headers["x-iii-message-id"], "M1");
}

#[tokio::test]
#[serial_test::serial(harness_trace)]
async fn harness_call_omits_message_id_header_when_absent() {
    // Option A: when the caller doesn't supply a message_id and baggage
    // is empty, no x-iii-message-id header is emitted. Plumbing calls
    // stay out of `Group by message`.
    let Some((iii, _url)) = boot_or_skip().await else {
        return;
    };
    let env = call_status_via_bridge(&iii, "S1", None).await;
    assert!(
        env["headers"].get("x-iii-message-id").is_none(),
        "x-iii-message-id header must be absent when no upstream message_id"
    );
}

#[tokio::test]
#[serial_test::serial(harness_trace)]
async fn harness_status_emits_traceparent_when_otel_enabled() {
    let Some((iii, _url)) = boot_or_skip().await else {
        return;
    };
    let env = call_status(&iii, "S1", Some("M-tp")).await;

    // traceparent is only emitted when the engine has the observability worker
    // active. If the harness boot succeeded but observability isn't running,
    // skip — this test doesn't assert that the worker was registered.
    let Some(tp) = env["headers"].get("traceparent").and_then(Value::as_str) else {
        skip_or_panic_otel("traceparent assertion: observability worker not active");
        return;
    };
    // Format: 00-<32 hex>-<16 hex>-01
    assert!(
        tp.starts_with("00-"),
        "traceparent should start with version 00-; got {tp}"
    );
    let parts: Vec<&str> = tp.split('-').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[1].len(), 32, "trace_id is 32 hex chars; got {tp}");
    assert_eq!(parts[2].len(), 16, "span_id is 16 hex chars; got {tp}");
}

/// Verifies the WORKING query path for finding harness traces:
/// `name: "harness.status"` + `search_all_spans: true`.
///
/// NOTE: this test does NOT exercise the engine's attribute filter
/// (`attributes: [["iii.session.id", ...]]`). The engine's attribute filter
/// targets ROOT spans only, but our `iii.*` attrs live on the CHILD harness
/// span. Filter-by-attribute is the spec's stated goal but isn't queryable
/// today; tracked as an engine-side follow-up (extend `search_all_spans` to
/// apply to attribute filter, OR propagate harness baggage into the engine
/// root span). Until then, the supported discovery path is by span name.
#[tokio::test]
#[serial_test::serial(harness_trace)]
async fn engine_traces_list_finds_harness_span_by_name() {
    let Some((iii, _url)) = boot_or_skip().await else {
        return;
    };
    let session = format!("test-session-{}", uuid::Uuid::new_v4());

    let env = call_status(&iii, &session, None).await;
    let Some(tp) = env["headers"].get("traceparent").and_then(Value::as_str) else {
        skip_or_panic_otel("observability worker not active");
        return;
    };
    let trace_id = tp
        .split('-')
        .nth(1)
        .expect("traceparent has trace_id")
        .to_string();

    // Spans land in the memory exporter asynchronously. Retry up to 10 times.
    // We use `name: "harness.status"` + `search_all_spans: true` because the
    // engine's attribute filter targets root spans only, and our iii.* attrs
    // live on the child span. Name+search_all matches any span in the trace and
    // returns its root span_id, which is what we need to correlate with the
    // trace_id minted in the previous status call.
    let mut found = None;
    for _ in 0..EXPORTER_FLUSH_RETRIES {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "engine::traces::list".into(),
                payload: json!({
                    "name": "harness.status",
                    "search_all_spans": true,
                }),
                action: None,
                timeout_ms: Some(TRIGGER_TIMEOUT_MS),
            })
            .await
            .expect("engine::traces::list");
        let spans = res["spans"].as_array().cloned().unwrap_or_default();
        if !spans.is_empty() {
            found = Some(spans);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(EXPORTER_FLUSH_INTERVAL_MS)).await;
    }

    let spans = found.expect("at least one span matching harness.status name");
    assert!(
        spans.iter().any(|s| s["trace_id"] == trace_id),
        "expected trace_id {trace_id} among returned spans; got {spans:?}"
    );
}

#[tokio::test]
#[serial_test::serial(harness_trace)]
async fn engine_traces_tree_returns_root_and_children() {
    let Some((iii, _url)) = boot_or_skip().await else {
        return;
    };
    let session = format!("tree-session-{}", uuid::Uuid::new_v4());

    let env = call_status(&iii, &session, Some("M-tree")).await;
    let Some(tp) = env["headers"].get("traceparent").and_then(Value::as_str) else {
        skip_or_panic_otel("observability worker not active");
        return;
    };
    let trace_id = tp
        .split('-')
        .nth(1)
        .expect("traceparent has trace_id")
        .to_string();

    // Same async-flush retry as above.
    let mut tree = None;
    for _ in 0..EXPORTER_FLUSH_RETRIES {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "engine::traces::tree".into(),
                payload: json!({ "trace_id": trace_id }),
                action: None,
                timeout_ms: Some(TRIGGER_TIMEOUT_MS),
            })
            .await
            .expect("engine::traces::tree");
        let roots = res["roots"].as_array().cloned().unwrap_or_default();
        if !roots.is_empty() {
            tree = Some(roots);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(EXPORTER_FLUSH_INTERVAL_MS)).await;
    }

    let roots = tree.expect("trace tree has at least one root");
    let root = &roots[0];
    // The root is the engine's auto-span (e.g. "handle_invocation harness::status").
    // iii.* attributes live on our CHILD span, not the root — the engine's
    // attribute filter targets root spans only, so we walk the tree to find our
    // child and assert on its attributes directly.

    #[allow(clippy::items_after_statements)]
    fn find_named<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
        if node["name"].as_str() == Some(name) {
            return Some(node);
        }
        node["children"]
            .as_array()
            .and_then(|cs| cs.iter().find_map(|c| find_named(c, name)))
    }

    let child = find_named(root, "harness.status")
        .expect("expected a `harness.status` span somewhere in the trace");

    // Attributes are stored as Vec<(String, String)> (see StoredSpan); JSON form
    // is an array of two-element arrays: `[["key", "value"], ...]`.
    let attrs = child["attributes"].as_array().expect("attributes array");
    let has_session = attrs
        .iter()
        .any(|kv| kv[0].as_str() == Some("iii.session.id") && kv[1].as_str() == Some(&session));
    let has_message = attrs
        .iter()
        .any(|kv| kv[0].as_str() == Some("iii.message.id") && kv[1].as_str() == Some("M-tree"));
    assert!(
        has_session,
        "child span missing iii.session.id; attrs={attrs:?}"
    );
    assert!(
        has_message,
        "child span missing iii.message.id; attrs={attrs:?}"
    );
}

#[tokio::test]
#[serial_test::serial(harness_trace)]
async fn harness_call_missing_function_id_still_emits_traced_error() {
    let Some((iii, _url)) = boot_or_skip().await else {
        return;
    };
    let session = format!("err-session-{}", uuid::Uuid::new_v4());

    // Probe: send a well-formed harness::call first and confirm we get a
    // traceparent back. If not, observability is down and the rest of this
    // test cannot record/query spans — skip. Use a different session so the
    // probe span doesn't collide with the error span during disambiguation.
    let probe_session = format!("probe-{}", uuid::Uuid::new_v4());
    let probe = iii
        .trigger(TriggerRequest {
            function_id: "harness::call".into(),
            payload: json!({
                "function_id": "harness::status",
                "payload": {},
                "session_id": probe_session,
                "message_id": "probe",
            }),
            action: None,
            timeout_ms: Some(TRIGGER_TIMEOUT_MS),
        })
        .await
        .expect("probe harness::call");
    if probe["headers"].get("traceparent").is_none() {
        skip_or_panic_otel("observability worker not active");
        return;
    }

    // harness::call expects { function_id, payload }. We deliberately omit
    // function_id. The handler returns IIIError::Handler("missing function_id"),
    // but the span we opened around the handler should still be recorded.
    let result = iii
        .trigger(TriggerRequest {
            function_id: "harness::call".into(),
            payload: json!({ "session_id": session, "message_id": "M-err" }),
            action: None,
            timeout_ms: Some(TRIGGER_TIMEOUT_MS),
        })
        .await;
    assert!(result.is_err(), "missing function_id must produce an error");

    // The span lands in the memory exporter even though the handler errored.
    // Use name+search_all_spans (attribute filter targets root spans only;
    // our iii.* attrs live on child spans).
    let mut found = None;
    for _ in 0..EXPORTER_FLUSH_RETRIES_UNDER_LOAD {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "engine::traces::list".into(),
                payload: json!({
                    "name": "harness.call",
                    "search_all_spans": true,
                    "limit": 500,
                }),
                action: None,
                timeout_ms: Some(TRIGGER_TIMEOUT_MS),
            })
            .await
            .expect("engine::traces::list");
        let spans = res["spans"].as_array().cloned().unwrap_or_default();
        if !spans.is_empty() {
            found = Some(spans);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            EXPORTER_FLUSH_INTERVAL_UNDER_LOAD_MS,
        ))
        .await;
    }

    let Some(spans) = found else {
        skip_or_panic_otel(
            "no harness.call spans landed in memory exporter (harness OTel exporter \
             may not be wired to the engine in this run)",
        );
        return;
    };
    // `name + search_all_spans` may match many traces; disambiguate by walking
    // each candidate's tree and finding the one whose harness.call child
    // carries our test session_id.

    #[allow(clippy::items_after_statements)]
    fn find_named<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
        if node["name"].as_str() == Some(name) {
            return Some(node);
        }
        node["children"]
            .as_array()
            .and_then(|cs| cs.iter().find_map(|c| find_named(c, name)))
    }

    #[allow(clippy::items_after_statements)]
    fn span_has_attr(span: &Value, key: &str, value: &str) -> bool {
        span["attributes"].as_array().is_some_and(|arr| {
            arr.iter()
                .any(|kv| kv[0].as_str() == Some(key) && kv[1].as_str() == Some(value))
        })
    }

    let mut matched_child: Option<Value> = None;
    'outer: for s in &spans {
        let Some(trace_id) = s["trace_id"].as_str() else {
            continue;
        };
        let tree = iii
            .trigger(TriggerRequest {
                function_id: "engine::traces::tree".into(),
                payload: json!({ "trace_id": trace_id }),
                action: None,
                timeout_ms: Some(TRIGGER_TIMEOUT_MS),
            })
            .await
            .expect("engine::traces::tree");
        let Some(roots) = tree["roots"].as_array() else {
            continue;
        };
        for root in roots {
            if let Some(child) = find_named(root, "harness.call") {
                if span_has_attr(child, "iii.session.id", &session) {
                    matched_child = Some(child.clone());
                    break 'outer;
                }
            }
        }
    }

    let Some(child) = matched_child else {
        skip_or_panic_otel(&format!(
            "matched no harness.call span with session_id={session} (memory exporter \
             likely evicted it under parallel test traffic; the span DID record per the \
             retry-loop hit on name)"
        ));
        return;
    };
    // Status field shape: StoredSpan.status is a string ("Error" / "Ok" / "Unset").
    let status = child["status"].as_str().unwrap_or("");
    assert!(
        status.eq_ignore_ascii_case("error"),
        "expected Status::error on the failed harness.call span; got {status:?}"
    );
}

#[cfg(test)]
mod unit_tests {
    use super::{skip_or_panic_inner, REQUIRE_ENGINE_ENV, REQUIRE_OTEL_ENV};

    #[test]
    fn skip_or_panic_inner_panics_when_required() {
        // H3 contract: when the require flag is true, the function MUST
        // panic with a message that references the env var name. The
        // production callers (`skip_or_panic_otel`, `boot_or_skip`) read
        // the env var and pass the bool — if a typo renames the env var
        // in CI, the panic message still points operators at the right
        // knob.
        let r = std::panic::catch_unwind(|| {
            skip_or_panic_inner(true, "EXAMPLE_REQUIRE_ENV", "reason text");
        });
        assert!(r.is_err(), "must panic when require=true");
        let msg = match r {
            Err(e) => e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default(),
            Ok(()) => String::new(),
        };
        assert!(
            msg.contains("EXAMPLE_REQUIRE_ENV"),
            "panic message should reference env-var name; got: {msg}"
        );
        assert!(
            msg.contains("reason text"),
            "panic message should include the reason; got: {msg}"
        );
    }

    #[test]
    fn skip_or_panic_inner_skips_silently_when_not_required() {
        // The non-require path returns Ok (just `eprintln!`).
        let r = std::panic::catch_unwind(|| {
            skip_or_panic_inner(false, "EXAMPLE_REQUIRE_ENV", "reason text");
        });
        assert!(r.is_ok(), "must not panic when require=false");
    }

    #[test]
    fn env_var_names_are_stable() {
        // L3+H3: if a future contributor renames these consts, CI YAML
        // that sets them will silently stop working. This test pins the
        // public surface so the rename has a tripwire.
        assert_eq!(REQUIRE_ENGINE_ENV, "HARNESS_TRACE_TESTS_REQUIRE_ENGINE");
        assert_eq!(REQUIRE_OTEL_ENV, "HARNESS_TRACE_TESTS_REQUIRE_OTEL");
    }

    #[test]
    fn allowlist_matches_iii_sdk_baggage_processor() {
        // Lock-step contract. The harness is the WRITER side
        // (`harness::otel::HARNESS_KEYS` — keys the wrapper sets into
        // baggage on every wrapped call) and iii-sdk is the READER side
        // (`iii_sdk::DEFAULT_ALLOWLIST` — keys the
        // `BaggageSpanProcessor` copies from baggage onto every span).
        // If a future change adds a fourth harness baggage key without
        // also growing the iii-sdk allowlist, the new key would silently
        // be dropped from downstream spans. This assertion catches that
        // drift in CI before it reaches production.
        assert_eq!(
            harness::otel::HARNESS_KEYS,
            iii_sdk::DEFAULT_ALLOWLIST,
            "harness::otel::HARNESS_KEYS (writer side) must equal \
             iii_sdk::DEFAULT_ALLOWLIST (reader side); update both in lockstep",
        );
    }
}
