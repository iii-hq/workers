//! The two change events this worker hosts: `memory::on-item-changed` and
//! `memory::on-bank-changed`.
//!
//! Each is a private function whose handler returns the event it is called
//! with. The worker calls it when the change happens, fire-and-forget
//! (`CallAction::Void`), and every observer whose filters match receives the
//! event. Neither takes args: an observer selects changes with filters on the
//! event (`input.bank`, `input.event_type`). The console memory page
//! live-refreshes off both:
//!
//! ```text
//! iii.observe(
//!     Source::function("memory::on-item-changed")
//!         .filters(json!({ "input.bank": { "$eq": "blog" } }))
//!         .capture([Capture::Output]),
//!     "my-worker::on-memory-changed",
//! )
//! ```

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::Memory;

pub const ON_EXTRACTION_DECISION_FN: &str = "memory::on-extraction-decision";
pub const ON_EXTRACTION_DECISION_DESC: &str =
    "Private operational terminal extraction decision, version 1. Only Memory emits after an active owned Harness completion activation and authoritative own SessionMeta lookup. No bank/content/model data. Observe output; filter input.session_id/turn_id. Delivery is best-effort (50ms), duplicates possible; loss is unmeasured, not success. Private producer ACL does not restrict observer readers: bounded session/turn identifiers are operational metadata, not secrets. This event alone proves neither bank unchanged nor CI success.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkippedSessionKind {
    Automation,
    E2e,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionDecisionStatus {
    Skipped,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionDecisionReason {
    SessionKindNotUser,
}

/// Operational evidence, not an extraction receipt or a bank-unchanged assertion.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractionDecisionEvent {
    /// Schema version, always 1.
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    #[schemars(length(min = 1, max = 256))]
    pub session_id: String,
    #[schemars(length(min = 1, max = 256))]
    pub turn_id: String,
    pub status: ExtractionDecisionStatus,
    pub reason: ExtractionDecisionReason,
    pub session_kind: SkippedSessionKind,
    /// Genuine Cargo package version, not a source/git identity.
    pub producer_version: String,
}

pub const ON_ITEM_CHANGED_FN: &str = "memory::on-item-changed";
pub const ON_ITEM_CHANGED_DESC: &str =
    "Called when a memory is created, updated, superseded, or deleted in a memory bank. \
     Returns the event: { event_type, bank, memory, reason? } (reason on consolidation changes, \
     and `redacted: <labels>` when secrets or invisible characters were removed on save/update). \
     Observe it for console/live views; filter on input.bank or input.event_type.";

pub const ON_BANK_CHANGED_FN: &str = "memory::on-bank-changed";
pub const ON_BANK_CHANGED_DESC: &str =
    "Called when a memory bank is created or trashed, or its rules change. Returns the event: \
     { event_type: created | trashed | rules-changed, bank }. Observe it; filter on input.bank \
     or input.event_type.";

/// Memory event kinds carried in `event_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ItemEvent {
    Created,
    Updated,
    Superseded,
    Deleted,
}

impl ItemEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemEvent::Created => "created",
            ItemEvent::Updated => "updated",
            ItemEvent::Superseded => "superseded",
            ItemEvent::Deleted => "deleted",
        }
    }
}

/// What `memory::on-item-changed` returns and its observers receive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ItemChangedEvent {
    pub event_type: ItemEvent,
    /// The bank the memory lives in.
    pub bank: String,
    /// The memory as stored after the change (a tombstone for `deleted`).
    pub memory: Memory,
    /// Why the change happened: `memory::supersede`'s reason, or
    /// `redacted: <labels>` when a save or update removed secrets or
    /// invisible characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What `memory::on-bank-changed` returns and its observers receive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BankChangedEvent {
    /// `created`, `trashed` or `rules-changed`.
    pub event_type: String,
    pub bank: String,
}

/// Register both change functions. Each is private (only this worker calls
/// it), takes no args, and returns the event it is called with.
pub fn register_all(iii: &IIIClient) {
    iii.register_function(
        ON_EXTRACTION_DECISION_FN,
        RegisterFunction::new(|event: ExtractionDecisionEvent| {
            if event.version != 1
                || !crate::extraction_proof::bounded(&event.session_id)
                || !crate::extraction_proof::bounded(&event.turn_id)
            {
                return Err(Error::Handler(
                    "invalid extraction decision correlation".into(),
                ));
            }
            Ok(event)
        })
        .description(ON_EXTRACTION_DECISION_DESC)
        .private(),
    );
    iii.register_function(
        ON_ITEM_CHANGED_FN,
        RegisterFunction::new(|event: ItemChangedEvent| Ok::<_, Error>(event))
            .description(ON_ITEM_CHANGED_DESC)
            .private(),
    );
    iii.register_function(
        ON_BANK_CHANGED_FN,
        RegisterFunction::new(|event: BankChangedEvent| Ok::<_, Error>(event))
            .description(ON_BANK_CHANGED_DESC)
            .private(),
    );
    tracing::info!(
        functions = ?[ON_ITEM_CHANGED_FN, ON_BANK_CHANGED_FN],
        "registered change functions"
    );
}

/// The `memory::on-item-changed` event: `{event_type, bank, memory, reason?}`
/// — `reason` only when the change carries one.
fn item_event(
    event: ItemEvent,
    bank: &str,
    memory: &Memory,
    reason: Option<&str>,
) -> ItemChangedEvent {
    ItemChangedEvent {
        event_type: event,
        bank: bank.to_string(),
        memory: memory.clone(),
        reason: reason.map(str::to_string),
    }
}

/// Calls the change functions when something observes them.
pub struct Emitter {
    iii: Arc<IIIClient>,
}

impl Emitter {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }

    /// Await only the bounded send, never an observer/model outcome. A lost
    /// event is unmeasured; repeated authenticated deliveries can duplicate it.
    pub async fn extraction_skipped(&self, session: &str, turn: &str, kind: SkippedSessionKind) {
        let event = ExtractionDecisionEvent {
            version: 1,
            session_id: session.into(),
            turn_id: turn.into(),
            status: ExtractionDecisionStatus::Skipped,
            reason: ExtractionDecisionReason::SessionKindNotUser,
            session_kind: kind,
            producer_version: env!("CARGO_PKG_VERSION").into(),
        };
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            self.emit(ON_EXTRACTION_DECISION_FN, serde_json::to_value(event)),
        )
        .await;
    }

    pub async fn item(&self, event: ItemEvent, bank: &str, memory: &Memory) {
        self.item_with(event, bank, memory, None).await;
    }

    /// `item` plus the why (`memory::supersede`'s `reason`), carried on the
    /// event so observers can explain automatic changes.
    pub async fn item_with(
        &self,
        event: ItemEvent,
        bank: &str,
        memory: &Memory,
        reason: Option<&str>,
    ) {
        if !self.iii.is_observed(ON_ITEM_CHANGED_FN) {
            return;
        }
        let payload = item_event(event, bank, memory, reason);
        self.emit(ON_ITEM_CHANGED_FN, serde_json::to_value(payload))
            .await;
    }

    pub async fn bank(&self, event_type: &str, bank: &str) {
        if !self.iii.is_observed(ON_BANK_CHANGED_FN) {
            return;
        }
        let payload = BankChangedEvent {
            event_type: event_type.to_string(),
            bank: bank.to_string(),
        };
        self.emit(ON_BANK_CHANGED_FN, serde_json::to_value(payload))
            .await;
    }

    /// Fire-and-forget call of `function_id`: the write that produced the
    /// change is not held by its observers, and a failure is logged and
    /// swallowed.
    async fn emit(&self, function_id: &str, payload: serde_json::Result<Value>) {
        let payload = match payload {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(function_id, error = %e, "change event did not serialize");
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
            tracing::warn!(function_id, error = %e, "change call failed");
        }
    }
}

// Registration (private, no args, the event as request and response) and
// delivery to bank-filtered observers are tested against an engine in
// tests/engine.rs.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn memory() -> Memory {
        Memory {
            id: "fp1".into(),
            text: "t".into(),
            entities: vec![],
            tags: vec![],
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: 1,
            updated_at: 1,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: None,
        }
    }

    #[test]
    fn item_event_carries_reason_only_when_given() {
        let m = memory();
        let with = serde_json::to_value(item_event(
            ItemEvent::Superseded,
            "b",
            &m,
            Some("same fact"),
        ))
        .unwrap();
        assert_eq!(with["event_type"], "superseded");
        assert_eq!(with["bank"], "b");
        assert_eq!(with["memory"]["id"], "fp1");
        assert_eq!(with["reason"], "same fact");
        let without = serde_json::to_value(item_event(ItemEvent::Updated, "b", &m, None)).unwrap();
        assert!(without.get("reason").is_none());
    }

    #[test]
    fn item_event_round_trips_through_the_handler_type() {
        let event = item_event(ItemEvent::Deleted, "main", &memory(), Some("why"));
        let wire = serde_json::to_value(&event).unwrap();
        let back: ItemChangedEvent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(back).unwrap(), wire);
    }

    #[test]
    fn bank_event_shape() {
        let event = BankChangedEvent {
            event_type: "rules-changed".into(),
            bank: "blog".into(),
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({ "event_type": "rules-changed", "bank": "blog" })
        );
    }
}
