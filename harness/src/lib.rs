//! Harness meta-worker. Composes the modular workers via `iii.worker.yaml`
//! runtime dependencies; this lib registers `harness::status` plus a
//! browser-facing `harness::call` HTTP route that forwards arbitrary
//! `{function_id, payload}` calls onto the iii bus.

pub mod fanout;
pub mod fs;
pub mod otel;

use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, Value,
    III,
};
use serde_json::json;

/// Upper bound for a single harness::call. Tool-calling turns roundtrip
/// the LLM 2+ times plus filesystem ops, so the engine's 30s default 504s
/// before turn-orchestrator finishes. 4 minutes covers the worst case we've
/// seen with Opus + a few tool calls.
const BRIDGE_TIMEOUT_MS: u64 = 240_000;

// `EXPECTED_WORKERS` is generated from `iii.worker.yaml` by `build.rs` so the
// Rust side and the engine-facing manifest cannot drift. Add or remove a
// worker by editing `iii.worker.yaml` only.
//
// Note: `iii-worker-manager` is intentionally NOT in `iii.worker.yaml`. It is
// provided by the iii engine itself (built-in default in the engine's
// `iii-worker/src/cli/builtin_defaults.rs`), not a discrete worker crate the
// harness can depend on. The browser SDK README's `iii worker add
// iii-worker-manager` instruction toggles that built-in feature, not a
// separate dependency.
include!(concat!(env!("OUT_DIR"), "/expected_workers.rs"));

pub struct HarnessFunctionRefs {
    pub status: FunctionRef,
    pub bridge: FunctionRef,
    pub bridge_info: FunctionRef,
    pub subscribe_fn: FunctionRef,
    pub unsubscribe_fn: FunctionRef,
    pub fanout_pumps: Option<fanout::FanoutPumps>,
    pub fs_read_inline: FunctionRef,
}

impl HarnessFunctionRefs {
    pub fn unregister_all(self) {
        self.status.unregister();
        self.bridge.unregister();
        self.bridge_info.unregister();
        self.subscribe_fn.unregister();
        self.unsubscribe_fn.unregister();
        self.fs_read_inline.unregister();
        if let Some(p) = self.fanout_pumps {
            p.shutdown();
        }
    }
}

/// Default engine URL used by `harness::info` when no override is provided.
/// Matches `harness/src/config.rs::default_engine_url`.
const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

/// Register harness functions with iii.
///
/// Uses [`DEFAULT_ENGINE_URL`] for the `engine_url` field of `harness::info`.
/// Production callers should prefer [`register_with_iii_with_engine_url`] so
/// the URL reflects the actual engine the harness is connected to.
pub async fn register_with_iii(iii: &III) -> anyhow::Result<HarnessFunctionRefs> {
    register_with_iii_with_engine_url(iii, DEFAULT_ENGINE_URL).await
}

#[allow(clippy::too_many_lines)]
pub async fn register_with_iii_with_engine_url(
    iii: &III,
    engine_url: &str,
) -> anyhow::Result<HarnessFunctionRefs> {
    let status = iii.register_function((
        RegisterFunctionMessage::with_id("harness::status".into()).with_description(
            "Returns the harness bundle name, version, and the list of expected runtime workers."
                .into(),
        ),
        |input: Value| async move {
            crate::otel::with_harness_span(
                "harness.status",
                input,
                crate::otel::IdSource::BodyOrTopLevel,
                crate::otel::WildcardMode::Disabled,
                None,
                |_input| async move {
                    Ok::<_, IIIError>(json!({
                        "ok": true,
                        "name": env!("CARGO_PKG_NAME"),
                        "version": env!("CARGO_PKG_VERSION"),
                        "expected_workers": EXPECTED_WORKERS,
                    }))
                },
            )
            .await
        },
    ));

    let iii_for_bridge = iii.clone();
    let bridge = iii.register_function((
        RegisterFunctionMessage::with_id("harness::call".into()).with_description(
            "Forward {function_id, payload} to iii.trigger and return the result. \
             Used by harness/web/ to reach the bus over HTTP."
                .into(),
        ),
        move |input: Value| {
            let iii = iii_for_bridge.clone();
            async move {
                let inner_function_id = input
                    .get("body")
                    .unwrap_or(&input)
                    .get("function_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                crate::otel::with_harness_span(
                    "harness.call",
                    input,
                    crate::otel::IdSource::BridgeTrigger,
                    crate::otel::WildcardMode::Disabled,
                    inner_function_id.as_deref(),
                    |input| async move {
                        let body = input.get("body").cloned().unwrap_or(input);
                        let function_id = body
                            .get("function_id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| IIIError::Handler("missing function_id".into()))?
                            .to_string();
                        let inner = body.get("payload").cloned().unwrap_or_else(|| json!({}));
                        let result = iii
                            .trigger(TriggerRequest {
                                function_id,
                                payload: inner,
                                action: None,
                                timeout_ms: Some(BRIDGE_TIMEOUT_MS),
                            })
                            .await
                            .map_err(|e| IIIError::Handler(e.to_string()))?;
                        Ok::<_, IIIError>(json!({
                            "status_code": 200,
                            "headers": { "content-type": "application/json" },
                            "body": result,
                        }))
                    },
                )
                .await
            }
        },
    ));

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "harness::call".into(),
        config: json!({
            "api_path": "harness/call",
            "http_method": "POST",
            "timeout_ms": BRIDGE_TIMEOUT_MS,
        }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // harness::info — relative WS path so reverse-proxy / HTTPS deployments
    // compose the full URL from `window.location`. `engine_url` is the direct
    // ws:// URL for callers (tests, native clients) that bypass the reverse
    // proxy.
    let engine_url_owned = engine_url.to_string();
    let bridge_info = iii.register_function((
        RegisterFunctionMessage::with_id("harness::info".into()).with_description(
            "Returns the relative WebSocket path and engine URL for browser clients.".into(),
        ),
        move |input: Value| {
            let engine_url = engine_url_owned.clone();
            async move {
                crate::otel::with_harness_span(
                    "harness.info",
                    input,
                    crate::otel::IdSource::BodyOrTopLevel,
                    crate::otel::WildcardMode::Disabled,
                    None,
                    |_input| async move {
                        Ok::<_, IIIError>(json!({
                            "ws_path": "/iii/ws",
                            "protocol": "ws",
                            "engine_url": engine_url,
                        }))
                    },
                )
                .await
            }
        },
    ));

    // Per-browser subscription registry. Skeleton: future steps will use this
    // to drive WS push of agent::events / state diffs / cost / approvals.
    let fanout = fanout::new_shared();

    let fanout_for_subscribe = Arc::clone(&fanout);
    let subscribe_fn = iii.register_function((
        RegisterFunctionMessage::with_id("ui::subscribe".into()).with_description(
            "Register a browser's interest in a session (or all sessions if session_id is null)."
                .into(),
        ),
        move |input: Value| {
            let fanout = Arc::clone(&fanout_for_subscribe);
            async move {
                crate::otel::with_harness_span(
                    "harness.ui.subscribe",
                    input,
                    crate::otel::IdSource::BodyOrTopLevel,
                    crate::otel::WildcardMode::OnExplicitNull,
                    None,
                    |input| async move {
                        let body = input.get("body").cloned().unwrap_or(input);
                        let browser_id = body
                            .get("browser_id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| IIIError::Handler("missing browser_id".into()))?
                            .to_string();
                        let session_id = body
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let total = {
                            let mut state = fanout.write().await;
                            state.subscribe(browser_id, session_id);
                            state.browser_count()
                        };
                        Ok::<_, IIIError>(json!({
                            "ok": true,
                            "total_browsers": total,
                        }))
                    },
                )
                .await
            }
        },
    ));

    let fanout_for_unsubscribe = Arc::clone(&fanout);
    let unsubscribe_fn = iii.register_function((
        RegisterFunctionMessage::with_id("ui::unsubscribe".into()).with_description(
            "Remove a browser's subscription to a session (or its all-sessions sub if session_id is null)."
                .into(),
        ),
        move |input: Value| {
            let fanout = Arc::clone(&fanout_for_unsubscribe);
            async move {
                crate::otel::with_harness_span(
                    "harness.ui.unsubscribe",
                    input,
                    crate::otel::IdSource::BodyOrTopLevel,
                    crate::otel::WildcardMode::OnExplicitNull,
                    None,
                    |input| async move {
                        let body = input.get("body").cloned().unwrap_or(input);
                        let browser_id = body
                            .get("browser_id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| IIIError::Handler("missing browser_id".into()))?
                            .to_string();
                        let session_id = body
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let total = {
                            let mut state = fanout.write().await;
                            state.unsubscribe(&browser_id, session_id);
                            state.browser_count()
                        };
                        Ok::<_, IIIError>(json!({
                            "ok": true,
                            "total_browsers": total,
                        }))
                    },
                )
                .await
            }
        },
    ));

    // Wire the upstream fanout pumps:
    //   - agent::events stream subscriber → ui::session::event::<browser_id>
    //   - state::list poll                → ui::sessions::changed::<browser_id>
    //
    // These spawn long-lived tasks; the returned handle ends them on
    // `unregister_all`. The `iii` clone here is fine — `III` is internally
    // ref-counted (it's already an `Arc<...>` inside the SDK).
    let fanout_pumps = fanout::spawn_subscribers(&Arc::new(iii.clone()), Arc::clone(&fanout));

    // harness::fs::read_inline — wraps shell::fs::read and inlines the
    // streamed bytes into the legacy {content,details} envelope so the
    // harness web FilesystemPanel preview keeps working without
    // channel-awareness.
    let iii_for_read = iii.clone();
    let engine_url_for_read = engine_url.to_string();
    let fs_read_inline = iii.register_function((
        RegisterFunctionMessage::with_id("harness::fs::read_inline".into()).with_description(
            "Read a host file via shell::fs::read, drain its channel, and \
             return a {content:[{text}], details:{size, truncated, bytes_read}} \
             envelope (max 256 KiB inline by default)."
                .into(),
        ),
        move |input: Value| {
            let iii = iii_for_read.clone();
            let ws_base = engine_url_for_read.clone();
            async move {
                Box::pin(crate::otel::with_harness_span(
                    "harness.fs.read_inline",
                    input,
                    crate::otel::IdSource::BodyOrTopLevel,
                    crate::otel::WildcardMode::Disabled,
                    None,
                    |input| async move {
                        let body = input.get("body").cloned().unwrap_or(input);
                        let args: fs::ReadInlineArgs = serde_json::from_value(body)
                            .map_err(|e| IIIError::Handler(format!("bad read_inline args: {e}")))?;
                        fs::read_inline(&iii, &ws_base, args).await
                    },
                ))
                .await
            }
        },
    ));

    Ok(HarnessFunctionRefs {
        status,
        bridge,
        bridge_info,
        subscribe_fn,
        unsubscribe_fn,
        fanout_pumps: Some(fanout_pumps),
        fs_read_inline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_workers_is_unique_and_non_empty() {
        assert!(!EXPECTED_WORKERS.is_empty());
        let mut seen = std::collections::HashSet::new();
        for w in EXPECTED_WORKERS {
            assert!(seen.insert(*w), "duplicate worker in EXPECTED_WORKERS: {w}");
        }
    }

    #[test]
    fn expected_workers_includes_approval_gate() {
        assert!(
            EXPECTED_WORKERS.contains(&"approval-gate"),
            "approval-gate is required for approval_required to actually block (Phase A item #3)"
        );
    }

    #[test]
    fn expected_workers_includes_shell_consolidated() {
        assert!(
            EXPECTED_WORKERS.contains(&"shell"),
            "consolidated `shell` worker must be in EXPECTED_WORKERS \
             (replaces shell-bash + shell-filesystem)"
        );
        assert!(
            !EXPECTED_WORKERS.contains(&"shell-bash"),
            "shell-bash was consolidated into `shell`"
        );
        assert!(
            !EXPECTED_WORKERS.contains(&"shell-filesystem"),
            "shell-filesystem was consolidated into `shell`"
        );
    }

    /// Manifest drift guard: the worker table in `harness/skill.md` must
    /// mention every entry in `EXPECTED_WORKERS`. The skill is hand-written
    /// (see `harness/skill.md`); when a worker is added to `iii.worker.yaml`
    /// the table must be updated in the same PR.
    #[test]
    fn harness_skill_md_lists_every_expected_worker() {
        const SKILL_MD: &str = include_str!("../skill.md");
        for w in EXPECTED_WORKERS {
            let needle = format!("`{w}`");
            assert!(
                SKILL_MD.contains(&needle),
                "harness/skill.md is missing an entry for worker {w:?} \
                 (look for `{w}` in the `## Workers` table)",
            );
        }
    }
}
