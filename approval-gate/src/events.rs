//! The two event functions this worker calls —
//! `approval::on-pending-created` / `approval::on-pending-resolved` — and
//! the [`Emitter`] behind them (the session-manager reactivity model:
//! consumers observe the function, the emitter calls it only while it is
//! observed, iii's delivery is fire-and-forget, at-most-once and unordered,
//! and `list_pending` is the reconciliation read).
//!
//! Both are private functions whose handlers return the event they are
//! called with: only this worker calls them, anyone may observe them, and
//! neither takes args. Consumers select calls with filters on the event:
//! `input.session_id`, and `input.session_metadata.<key>` for the tenancy
//! keys the record denormalizes from its session.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use serde::Serialize;

use crate::types::{PendingApprovalRecord, PendingCreatedEvent, PendingResolvedEvent};

pub const ON_PENDING_CREATED_FN: &str = "approval::on-pending-created";
pub const ON_PENDING_CREATED_DESC: &str =
    "Called when a function call was held for human approval and its inbox record written. \
     Event: PendingApprovalRecord with status \"pending\" (redacted args, session context). \
     No args; filter on input.session_id or input.session_metadata.<key>. Observe it from \
     notification workers.";

pub const ON_PENDING_RESOLVED_FN: &str = "approval::on-pending-resolved";
pub const ON_PENDING_RESOLVED_DESC: &str =
    "Called when a pending approval left the inbox (outcome: allow | deny | aborted). \
     Called exactly once per record; lets UIs clear badges. No args; filter on \
     input.session_id or input.session_metadata.<key>.";

/// Register both event functions: private, no args, each handler returning
/// the event it is called with. Runs before `functions::register_all`, whose
/// handlers call them through the [`Emitter`].
pub fn register_functions(iii: &IIIClient) {
    iii.register_function(
        ON_PENDING_CREATED_FN,
        RegisterFunction::new(|event: PendingCreatedEvent| Ok::<_, Error>(event))
            .description(ON_PENDING_CREATED_DESC)
            .private(),
    );
    iii.register_function(
        ON_PENDING_RESOLVED_FN,
        RegisterFunction::new(|event: PendingResolvedEvent| Ok::<_, Error>(event))
            .description(ON_PENDING_RESOLVED_DESC)
            .private(),
    );
    tracing::info!(
        functions = ?[ON_PENDING_CREATED_FN, ON_PENDING_RESOLVED_FN],
        "registered event functions"
    );
}

/// Where emissions go. Production calls the event functions; tests record.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn pending_created(&self, record: &PendingApprovalRecord);
    async fn pending_resolved(&self, event: &PendingResolvedEvent);
}

/// Fire-and-forget calls of the event functions, made only while something
/// observes them.
pub struct Emitter {
    iii: Arc<IIIClient>,
}

impl Emitter {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }

    async fn emit<T: Serialize>(&self, function_id: &'static str, event: &T) {
        if !self.iii.is_observed(function_id) {
            return;
        }
        let payload = match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(function_id, error = %e, "event failed to serialize");
                return;
            }
        };
        let request = CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(e) = self.iii.call(request).await {
            tracing::warn!(function_id, error = %e, "void event call failed");
        }
    }
}

#[async_trait]
impl EventSink for Emitter {
    async fn pending_created(&self, record: &PendingApprovalRecord) {
        self.emit(
            ON_PENDING_CREATED_FN,
            &PendingCreatedEvent::pending(record.clone()),
        )
        .await;
    }

    async fn pending_resolved(&self, event: &PendingResolvedEvent) {
        self.emit(ON_PENDING_RESOLVED_FN, event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn descriptions_name_the_filterable_fields() {
        for description in [ON_PENDING_CREATED_DESC, ON_PENDING_RESOLVED_DESC] {
            assert!(description.contains("input.session_id"), "{description}");
            assert!(
                description.contains("input.session_metadata.<key>"),
                "{description}"
            );
        }
    }

    #[tokio::test]
    async fn nothing_is_called_while_unobserved() {
        // `IIIClient::new` builds local state only: no engine, no observers.
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        register_functions(&iii);
        assert!(!iii.is_observed(ON_PENDING_CREATED_FN));
        assert!(!iii.is_observed(ON_PENDING_RESOLVED_FN));
        let emitter = Emitter::new(iii);
        // Returns at once: an unobserved call never reaches the connection.
        emitter
            .pending_resolved(&PendingResolvedEvent {
                session_id: "s_1".into(),
                turn_id: "t_1".into(),
                function_call_id: "c_1".into(),
                function_id: "shell::run".into(),
                outcome: crate::types::ResolvedOutcome::Allow,
                reason: None,
                session_metadata: None,
                resolved_at: 1,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn emitter_delivers_created_events_with_status() {
        crate::testkit::with_stack(
            crate::testkit::BootOpts::needs_approval(),
            |stack| async move {
                let out = crate::testkit::call(
                    &stack.iii,
                    "approval::gate",
                    crate::testkit::hook_input("s_1", "c_1", "shell::run"),
                )
                .await
                .unwrap();
                assert_eq!(out["decision"], json!("hold"));
                assert!(
                    crate::testkit::wait_for(3_000, || {
                        !crate::testkit::log_snapshot(&stack.created).is_empty()
                    })
                    .await
                );
                let created = crate::testkit::log_snapshot(&stack.created);
                assert_eq!(created.len(), 1);
                assert_eq!(created[0]["status"], json!("pending"));
                assert_eq!(created[0]["session_id"], json!("s_1"));
            },
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn emitter_delivers_resolved_events() {
        crate::testkit::with_stack(
            crate::testkit::BootOpts::needs_approval(),
            |stack| async move {
                crate::testkit::state_set(
                    &stack.iii,
                    crate::pending::PENDING_SCOPE,
                    "s_1/c_1",
                    json!({
                        "session_id": "s_1",
                        "turn_id": "t_1",
                        "function_call_id": "c_1",
                        "function_id": "shell::run",
                        "kind": "function",
                        "arguments_excerpt": {},
                        "pending_at": 100,
                        "depth": 0,
                    }),
                )
                .await;
                crate::testkit::call(
                    &stack.iii,
                    "approval::resolve",
                    json!({
                        "session_id": "s_1",
                        "function_call_id": "c_1",
                        "decision": "allow"
                    }),
                )
                .await
                .unwrap();
                assert!(
                    crate::testkit::wait_for(3_000, || {
                        !crate::testkit::log_snapshot(&stack.resolved).is_empty()
                    })
                    .await
                );
                let resolved = crate::testkit::log_snapshot(&stack.resolved);
                assert_eq!(resolved[0]["outcome"], json!("allow"));
            },
        )
        .await;
    }
}
