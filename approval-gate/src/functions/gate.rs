//! `approval::gate` — the `pre_dispatch` hook (approval-gate.md § The
//! approval::gate hook). Maps `HookInput` → `HookOutput`. Never errors:
//! every failure mode resolves to a fail-closed `deny` so a confused
//! harness cannot interpret an exception as anything but denial (its
//! `on_error: fail_closed` would do the same).
//!
//! Runs inside the harness's at-least-once steps and is idempotent: a
//! redelivered step re-runs the gate for the same `function_call_id`;
//! the pending-record write is keyed on it, so a duplicate hold is a
//! no-op on the existing record (and emits no second `pending_created`).

use serde_json::json;

use super::Deps;
use crate::decision;
use crate::denial::{gate_unavailable_envelope, human_only_denial, permissions_deny_envelope};
use crate::error::ApprovalError;
use crate::gate_config::{snapshot, GateDefaults};
use crate::pending;
use crate::policy::{self, PolicyOutcome};
use crate::redact::redact;
use crate::settings;
use crate::types::{
    now_ms, validate_id, HookCall, HookInput, HookOutput, JsonMap, PendingApprovalRecord,
};

pub async fn handle(deps: &Deps, input: HookInput) -> Result<HookOutput, ApprovalError> {
    let Some(call) = input.call.clone() else {
        return Ok(deny(
            "approval-gate received a pre_dispatch hook input without a call payload",
        ));
    };

    // 1. Human-only defense — before the settings snapshot, before
    //    policy, and regardless of mode (even `full`).
    if decision::is_human_only(&call.function_id) {
        let envelope = human_only_denial(&call.function_id, &call.arguments);
        return Ok(deny(&envelope.reason));
    }

    // A call whose ids can't key a pending record can never be held —
    // and a malformed id is a boundary violation anyway. Fail closed.
    if validate_id("session_id", &input.session_id).is_err()
        || validate_id("function_call_id", &call.id).is_err()
    {
        return Ok(deny(&format!(
            "approval-gate cannot evaluate {}: session_id / function_call_id must be non-empty and must not contain \"/\"",
            call.function_id
        )));
    }

    // 2. One settings snapshot per call (race-safe). A state outage
    //    degrades to the configuration defaults: safe, because the
    //    default never widens beyond what the deployment configured.
    let defaults = snapshot(&deps.defaults);
    let stored = settings::read_tolerant(
        deps.bus.as_ref(),
        &input.session_id,
        deps.cfg.state_timeout_ms,
    )
    .await;
    let (effective, _) = settings::effective(stored, &defaults);

    // 3-5. Mode / allow-list short-circuits.
    if decision::pre_policy_allow(&effective, &call.function_id) {
        return Ok(HookOutput::Continue);
    }

    // 6. Yaml policy fallback.
    match policy::check(
        deps.bus.as_ref(),
        &call.function_id,
        &call.arguments,
        deps.cfg.policy_timeout_ms,
    )
    .await
    {
        PolicyOutcome::Allow => Ok(HookOutput::Continue),
        PolicyOutcome::Deny {
            rule_id,
            matched_constraint,
        } => {
            let envelope = permissions_deny_envelope(
                &call.function_id,
                &rule_id,
                matched_constraint,
                &call.arguments,
            );
            Ok(deny(&envelope.reason))
        }
        PolicyOutcome::Unavailable(why) => {
            let envelope = gate_unavailable_envelope(&call.function_id, &why);
            Ok(deny(&envelope.reason))
        }
        PolicyOutcome::NeedsApproval => Ok(hold(deps, &input, &call, &defaults).await),
    }
}

fn deny(reason: &str) -> HookOutput {
    HookOutput::Deny {
        reason: reason.to_string(),
    }
}

/// The hold path. The pending record is written **synchronously, before
/// the hook returns hold** — a held call must never be invisible to the
/// inbox. Write failure → fail-closed deny, never hold blind.
/// `pending_created` emits asynchronously after the record is written —
/// notification fan-out never blocks the dispatch hot path.
async fn hold(
    deps: &Deps,
    input: &HookInput,
    call: &HookCall,
    defaults: &GateDefaults,
) -> HookOutput {
    let bus = deps.bus.as_ref();

    // Idempotency: a redelivered step re-holds the same call.
    match pending::get(bus, &input.session_id, &call.id, deps.cfg.state_timeout_ms).await {
        Ok(Some(existing)) => {
            return HookOutput::Hold {
                pending_timeout_ms: (existing.expires_at - existing.pending_at).max(0),
            };
        }
        Ok(None) => {}
        Err(e) => {
            let envelope = gate_unavailable_envelope(
                &call.function_id,
                &format!("pending record read failed: {e}"),
            );
            return deny(&envelope.reason);
        }
    }

    let (session_title, session_description, session_metadata) =
        fetch_session_context(deps, &input.session_id).await;

    let pending_at = now_ms();
    let record = PendingApprovalRecord {
        session_id: input.session_id.clone(),
        turn_id: input.turn_id.clone(),
        function_call_id: call.id.clone(),
        function_id: call.function_id.clone(),
        arguments_excerpt: redact(&call.arguments),
        pending_at,
        expires_at: pending_at + defaults.pending_timeout_ms,
        session_title,
        session_description,
        session_metadata,
        depth: input.depth,
        assistant_excerpt: None,
    };

    match pending::put(bus, &record, deps.cfg.state_timeout_ms).await {
        Err(e) => {
            let envelope = gate_unavailable_envelope(
                &call.function_id,
                &format!("pending record write failed: {e}"),
            );
            deny(&envelope.reason)
        }
        // Lost a write race with a concurrent duplicate hold: the first
        // writer's record (and emission) stands.
        Ok(Some(_prior)) => HookOutput::Hold {
            pending_timeout_ms: defaults.pending_timeout_ms,
        },
        Ok(None) => {
            let sink = deps.sink.clone();
            tokio::spawn(async move {
                sink.pending_created(&record).await;
            });
            HookOutput::Hold {
                pending_timeout_ms: defaults.pending_timeout_ms,
            }
        }
    }
}

/// Best-effort `session::get` — fields are omitted on any failure
/// (session-manager absent, timeout, unknown session), within its own
/// budget so it can't eat the hook's.
async fn fetch_session_context(
    deps: &Deps,
    session_id: &str,
) -> (Option<String>, Option<String>, Option<JsonMap>) {
    let reply = deps
        .bus
        .call(
            "session::get",
            json!({ "session_id": session_id }),
            Some(deps.cfg.session_fetch_timeout_ms),
        )
        .await;
    let Ok(reply) = reply else {
        return (None, None, None);
    };
    let Some(meta) = reply.get("meta") else {
        return (None, None, None);
    };
    let title = meta
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    let description = meta
        .get("description")
        .and_then(serde_json::Value::as_str)
        .filter(|d| !d.is_empty())
        .map(str::to_string);
    let metadata = meta
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .cloned();
    (title, description, metadata)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::{replace, shared_defaults};
    use crate::pending::PENDING_SCOPE;
    use crate::settings::SETTINGS_SCOPE;
    use crate::testkit::{FakeBus, MemoryState};
    use crate::types::{ApprovalSettings, PermissionMode};

    struct Fixture {
        deps: Arc<Deps>,
        bus: Arc<FakeBus>,
        sink: Arc<RecordingSink>,
        state: MemoryState,
    }

    fn fixture() -> Fixture {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let sink = Arc::new(RecordingSink::new());
        let deps = Arc::new(Deps {
            bus: bus.clone(),
            sink: sink.clone(),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        Fixture {
            deps,
            bus,
            sink,
            state,
        }
    }

    fn seed_settings(f: &Fixture, session_id: &str, settings: &ApprovalSettings) {
        f.state.seed(
            SETTINGS_SCOPE,
            session_id,
            serde_json::to_value(settings).unwrap(),
        );
    }

    fn grants(ids: &[&str]) -> Vec<crate::types::AlwaysAllowEntry> {
        ids.iter()
            .map(|id| crate::types::AlwaysAllowEntry {
                function_id: id.to_string(),
                granted_at: 0,
                granted_by: crate::types::GrantedBy::UserClick,
            })
            .collect()
    }

    fn hook_input(function_id: &str) -> HookInput {
        serde_json::from_value(json!({
            "point": "pre_dispatch",
            "session_id": "s_1",
            "turn_id": "t_1",
            "step": 1,
            "depth": 0,
            "call": { "id": "c_1", "function_id": function_id, "arguments": { "cmd": "ls" } }
        }))
        .unwrap()
    }

    fn policy_needs_approval(f: &Fixture) {
        f.bus.on_value(
            "policy::check_permissions",
            json!({ "decision": "needs_approval" }),
        );
    }

    async fn run(f: &Fixture, function_id: &str) -> HookOutput {
        handle(&f.deps, hook_input(function_id)).await.unwrap()
    }

    fn settle() -> tokio::time::Sleep {
        // Let the spawned pending_created emission run.
        tokio::time::sleep(std::time::Duration::from_millis(20))
    }

    // --- The seven prior-art permission-matrix cases -------------------

    #[tokio::test]
    async fn case_1_full_mode_allows_without_consulting_policy() {
        let f = fixture();
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Full,
                ..Default::default()
            },
        );
        assert_eq!(run(&f, "shell::run").await, HookOutput::Continue);
        assert!(f.bus.calls_to("policy::check_permissions").is_empty());
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
    }

    #[tokio::test]
    async fn case_2_approved_always_holds_in_manual_mode() {
        let f = fixture();
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Manual,
                approved_always: grants(&["shell::run"]),
                ..Default::default()
            },
        );
        assert_eq!(run(&f, "shell::run").await, HookOutput::Continue);
        assert!(f.bus.calls_to("policy::check_permissions").is_empty());
    }

    #[tokio::test]
    async fn case_3_auto_mode_always_allow_hit_allows() {
        let f = fixture();
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Auto,
                always_allow: grants(&["shell::run"]),
                ..Default::default()
            },
        );
        assert_eq!(run(&f, "shell::run").await, HookOutput::Continue);
    }

    #[tokio::test]
    async fn case_4_manual_mode_without_grants_holds() {
        let f = fixture();
        policy_needs_approval(&f);
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Manual,
                ..Default::default()
            },
        );
        let out = run(&f, "shell::run").await;
        assert_eq!(
            out,
            HookOutput::Hold {
                pending_timeout_ms: 1_800_000
            }
        );
        let record = f.state.peek(PENDING_SCOPE, "s_1/c_1").unwrap();
        assert_eq!(record["function_id"], json!("shell::run"));
        settle().await;
        assert_eq!(f.sink.created_events().len(), 1);
    }

    #[tokio::test]
    async fn case_5_always_allow_is_dormant_in_manual_mode() {
        let f = fixture();
        policy_needs_approval(&f);
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Manual,
                always_allow: grants(&["shell::run"]),
                ..Default::default()
            },
        );
        assert!(matches!(
            run(&f, "shell::run").await,
            HookOutput::Hold { .. }
        ));
    }

    #[tokio::test]
    async fn case_6_auto_mode_miss_holds() {
        let f = fixture();
        policy_needs_approval(&f);
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Auto,
                always_allow: grants(&["other::fn"]),
                ..Default::default()
            },
        );
        assert!(matches!(
            run(&f, "shell::run").await,
            HookOutput::Hold { .. }
        ));
    }

    #[tokio::test]
    async fn case_7_self_escalation_is_denied_before_everything() {
        let f = fixture();
        // Even under full mode, and without policy or state scripted.
        seed_settings(
            &f,
            "s_1",
            &ApprovalSettings {
                mode: PermissionMode::Full,
                ..Default::default()
            },
        );
        for target in [
            "approval::set_mode",
            "approval::resolve",
            "configuration::set",
        ] {
            let out = run(&f, target).await;
            let HookOutput::Deny { reason } = out else {
                panic!("expected deny for {target}");
            };
            assert!(reason.contains("human_only_function"), "{reason}");
        }
        assert!(f.bus.calls_to("policy::check_permissions").is_empty());
    }

    // --- Greenfield rows ------------------------------------------------

    #[tokio::test]
    async fn policy_allow_continues() {
        let f = fixture();
        f.bus.on_value(
            "policy::check_permissions",
            json!({ "decision": "allow", "rule_id": "r1" }),
        );
        assert_eq!(run(&f, "shell::run").await, HookOutput::Continue);
    }

    #[tokio::test]
    async fn policy_deny_carries_the_permissions_reason() {
        let f = fixture();
        f.bus.on_value(
            "policy::check_permissions",
            json!({
                "decision": "deny",
                "rule_id": "no_shell",
                "matched_constraint": { "field": "cmd", "operator": "eq", "value": "ls" }
            }),
        );
        let HookOutput::Deny { reason } = run(&f, "shell::run").await else {
            panic!("expected deny");
        };
        assert!(reason.contains("matched rule no_shell"));
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
    }

    #[tokio::test]
    async fn garbage_policy_reply_degrades_to_hold() {
        let f = fixture();
        f.bus
            .on_value("policy::check_permissions", json!("what even is this"));
        assert!(matches!(
            run(&f, "shell::run").await,
            HookOutput::Hold { .. }
        ));
    }

    #[tokio::test]
    async fn policy_transport_failure_fails_closed() {
        let f = fixture();
        f.bus
            .on_error("policy::check_permissions", "connection refused");
        let HookOutput::Deny { reason } = run(&f, "shell::run").await else {
            panic!("expected deny");
        };
        assert!(reason.contains("policy unreachable"));
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
    }

    #[tokio::test]
    async fn missing_policy_worker_fails_closed() {
        let bus = Arc::new(FakeBus::new());
        let _state = bus.with_memory_state();
        let sink = Arc::new(RecordingSink::new());
        let deps = Arc::new(Deps {
            bus: bus.clone(),
            sink,
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        let out = handle(&deps, hook_input("shell::run")).await.unwrap();
        assert!(matches!(out, HookOutput::Deny { .. }));
    }

    #[tokio::test]
    async fn pending_write_failure_denies_never_holds_blind() {
        let f = fixture();
        policy_needs_approval(&f);
        // State reads succeed (no record) but writes fail.
        f.bus.on("state::set", |_| {
            Err(crate::bus::BusError("disk full".into()))
        });
        let HookOutput::Deny { reason } = run(&f, "shell::run").await else {
            panic!("expected deny");
        };
        assert!(reason.contains("pending record write failed"));
        settle().await;
        assert!(f.sink.created_events().is_empty());
    }

    #[tokio::test]
    async fn duplicate_hold_is_a_noop_on_the_existing_record() {
        let f = fixture();
        policy_needs_approval(&f);
        let first = run(&f, "shell::run").await;
        assert!(matches!(first, HookOutput::Hold { .. }));
        let before = f.state.peek(PENDING_SCOPE, "s_1/c_1").unwrap();

        let second = run(&f, "shell::run").await;
        assert!(matches!(second, HookOutput::Hold { .. }));
        // No rewrite: pending_at unchanged.
        assert_eq!(f.state.peek(PENDING_SCOPE, "s_1/c_1").unwrap(), before);
        settle().await;
        assert_eq!(f.sink.created_events().len(), 1);
    }

    #[tokio::test]
    async fn no_stored_settings_uses_configuration_defaults() {
        let f = fixture();
        // Deployment defaults: auto mode with a seeded glob.
        replace(
            &f.deps.defaults,
            crate::gate_config::GateDefaults {
                default_mode: PermissionMode::Auto,
                always_allow_seed: vec!["shell::*".into()],
                pending_timeout_ms: 60_000,
            },
        );
        // Seed honored in auto (no stored record).
        assert_eq!(run(&f, "shell::run").await, HookOutput::Continue);

        // Same seed dormant under manual defaults.
        replace(
            &f.deps.defaults,
            crate::gate_config::GateDefaults {
                default_mode: PermissionMode::Manual,
                always_allow_seed: vec!["shell::*".into()],
                pending_timeout_ms: 60_000,
            },
        );
        policy_needs_approval(&f);
        let out = run(&f, "shell::run").await;
        assert_eq!(
            out,
            HookOutput::Hold {
                pending_timeout_ms: 60_000
            }
        );
    }

    #[tokio::test]
    async fn record_carries_redacted_args_and_session_context() {
        let f = fixture();
        policy_needs_approval(&f);
        f.bus.on_value(
            "session::get",
            json!({ "meta": {
                "session_id": "s_1",
                "title": "Deploy review",
                "description": "prod deploy",
                "metadata": { "owner": "u_1" }
            }}),
        );
        let input: HookInput = serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "depth": 2,
            "call": {
                "id": "c_1",
                "function_id": "shell::run",
                "arguments": { "cmd": "ls", "api_key": "sk_live_123" }
            }
        }))
        .unwrap();
        handle(&f.deps, input).await.unwrap();

        let record = f.state.peek(PENDING_SCOPE, "s_1/c_1").unwrap();
        assert_eq!(record["arguments_excerpt"]["api_key"], json!("<redacted>"));
        assert_eq!(record["session_title"], json!("Deploy review"));
        assert_eq!(record["session_metadata"]["owner"], json!("u_1"));
        assert_eq!(record["depth"], json!(2));
        assert_eq!(
            record["expires_at"].as_i64().unwrap() - record["pending_at"].as_i64().unwrap(),
            1_800_000
        );
    }

    #[tokio::test]
    async fn session_fetch_failure_omits_context_but_still_holds() {
        let f = fixture();
        policy_needs_approval(&f);
        // No session::get scripted → transport error → fields omitted.
        assert!(matches!(
            run(&f, "shell::run").await,
            HookOutput::Hold { .. }
        ));
        let record = f.state.peek(PENDING_SCOPE, "s_1/c_1").unwrap();
        assert!(record.get("session_title").is_none());
        assert!(record.get("session_metadata").is_none());
    }

    #[tokio::test]
    async fn slash_in_ids_fails_closed() {
        let f = fixture();
        let input: HookInput = serde_json::from_value(json!({
            "session_id": "s/1",
            "turn_id": "t_1",
            "call": { "id": "c_1", "function_id": "shell::run", "arguments": {} }
        }))
        .unwrap();
        assert!(matches!(
            handle(&f.deps, input).await.unwrap(),
            HookOutput::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn missing_call_payload_fails_closed() {
        let f = fixture();
        let input: HookInput =
            serde_json::from_value(json!({ "session_id": "s_1", "turn_id": "t_1" })).unwrap();
        assert!(matches!(
            handle(&f.deps, input).await.unwrap(),
            HookOutput::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn settings_state_outage_degrades_to_defaults_not_allow() {
        let f = fixture();
        // state::get fails entirely; manual defaults → policy consult.
        f.bus
            .on("state::get", |_| Err(crate::bus::BusError("down".into())));
        f.bus.on_error("policy::check_permissions", "also down");
        // Fail-closed end to end: deny, not allow, not hold.
        assert!(matches!(
            run(&f, "shell::run").await,
            HookOutput::Deny { .. }
        ));
    }
}
