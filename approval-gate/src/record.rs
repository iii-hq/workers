//! Approval-gate record schema.
//!
//! `Pending → InFlight → Done(Outcome)`. The intermediate InFlight write
//! between operator-approve and the executor `iii.trigger` is what closes
//! the duplicate-execution race — a second `approval::resolve` arriving
//! during the invoke await sees a non-Pending row and bails.
//!
//! `lifecycle.rs` is gone; its only surviving helper
//! (`flipped_to_timed_out_if_expired`) lives here as a `Record` method
//! because it operates on a `Record`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rules::RuleMatch;
use crate::wire::Denial;

/// How long an `InFlight` row may sit before lazy-flip reclaims it as
/// `Done(TimedOut)`. Chosen so a legitimately slow invoke is not stolen
/// from its caller, while a wedged or persistence-lost row still has a
/// bounded orphan window. The reclaim covers finding #5: an executor
/// invoke that succeeded but whose Done write failed leaves a row in
/// InFlight indefinitely otherwise.
pub const IN_FLIGHT_GRACE_MS: u64 = 600_000;

/// Lifecycle status. Wire format is snake_case so iii-state dumps stay
/// human-readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting for the operator's decision (no outcome attached).
    Pending,
    /// Operator approved; underlying `iii.trigger` is in flight. Persisted
    /// to close the dup-exec race across concurrent `approval::resolve`
    /// calls within a worker process.
    InFlight,
    /// Terminal. `outcome` is `Some`.
    Done,
}

/// Outcome data attached to terminal records. Tagged enum on the wire
/// (`{ "kind": "...", "detail": { ... } }`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Outcome {
    Executed { result: Value },
    Failed { error: String },
    Denied { denial: Denial },
    TimedOut,
}

/// Persisted approval record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub function_call_id: String,
    pub function_id: String,
    pub args: Value,
    pub session_id: String,
    pub expires_at: u64,
    pub status: Status,
    /// `Some` iff `status == Done`. Constructors enforce this invariant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Unix ms stamped on the first non-Pending transition. `handle_consume`
    /// sorts entries by this field so multi-row consumes (cascade case)
    /// produce deterministic LLM message order. Provider-minted
    /// `function_call_id` (Anthropic `toolu_*`, OpenAI `call_*`) is not
    /// lex-monotonic and can't substitute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<RuleMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Record {
    /// Fresh Pending row. `expires_at = now_ms + timeout_ms`, saturating on
    /// overflow so a buggy caller can't underflow the deadline.
    pub fn pending(
        function_call_id: String,
        function_id: String,
        args: Value,
        session_id: String,
        now_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        Self {
            function_call_id,
            function_id,
            args,
            session_id,
            expires_at: now_ms.saturating_add(timeout_ms),
            status: Status::Pending,
            outcome: None,
            resolved_at: None,
            rule: None,
            reason: None,
        }
    }

    pub fn pending_with_rule(
        function_call_id: String,
        function_id: String,
        args: Value,
        session_id: String,
        now_ms: u64,
        timeout_ms: u64,
        rule: Option<RuleMatch>,
    ) -> Self {
        let reason = rule.as_ref().and_then(|r| r.reason.clone());
        Self {
            rule,
            reason,
            ..Self::pending(
                function_call_id,
                function_id,
                args,
                session_id,
                now_ms,
                timeout_ms,
            )
        }
    }

    /// Pending → InFlight. Stamps `resolved_at` (the "first non-Pending"
    /// marker for ordering). Caller is responsible for ensuring the row
    /// was actually Pending before calling; this is enforced at the
    /// callsite (`handle_resolve`) via a Status check.
    pub fn in_flight(self, now_ms: u64) -> Self {
        Self {
            status: Status::InFlight,
            resolved_at: Some(self.resolved_at.unwrap_or(now_ms)),
            ..self
        }
    }

    /// InFlight → Done. Preserves `resolved_at` from the InFlight write
    /// (so audit timestamps reflect when the row left Pending, not when
    /// the invoke finished).
    pub fn done(self, outcome: Outcome) -> Self {
        Self {
            status: Status::Done,
            outcome: Some(outcome),
            ..self
        }
    }

    /// Pending → Done directly (deny path, timeout flip — paths that
    /// don't run an invoke). Stamps `resolved_at` with `now_ms`.
    pub fn done_at(self, now_ms: u64, outcome: Outcome) -> Self {
        Self {
            status: Status::Done,
            outcome: Some(outcome),
            resolved_at: Some(self.resolved_at.unwrap_or(now_ms)),
            ..self
        }
    }

    /// Lazy timeout flip. Returns `Some(flipped)` for two cases:
    ///
    /// 1. **Pending row past `expires_at`** — the operator never resolved
    ///    in time. Flips to `Done(TimedOut)`.
    /// 2. **InFlight row past `resolved_at + IN_FLIGHT_GRACE_MS`** — the
    ///    invoke either succeeded with a lost Done write (see finding #5)
    ///    or wedged inside the function executor. Flips to
    ///    `Done(TimedOut)` so an external reclaim path can drain the
    ///    orphan into the LLM history instead of leaving a permanent
    ///    "Awaiting human approval" placeholder. The grace is generous
    ///    so that a legitimately slow invoke is not stolen.
    ///
    /// Done rows are already terminal and never touched.
    pub fn flipped_to_timed_out_if_expired(&self, now_ms: u64) -> Option<Record> {
        match self.status {
            Status::Pending if now_ms >= self.expires_at => {
                Some(self.clone().done_at(now_ms, Outcome::TimedOut))
            }
            Status::InFlight => {
                let resolved_at = self.resolved_at?;
                let reclaim_at = resolved_at.saturating_add(IN_FLIGHT_GRACE_MS);
                if now_ms >= reclaim_at {
                    Some(Record {
                        status: Status::Done,
                        outcome: Some(Outcome::TimedOut),
                        ..self.clone()
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Wire JSON shape (infallible — only serializable fields).
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("Record is always serializable")
    }

    /// Parse from wire JSON. `None` means the row doesn't match the
    /// schema; callers skip such rows.
    pub fn from_value(v: Value) -> Option<Self> {
        serde_json::from_value(v).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pending_record() -> Record {
        Record::pending(
            "tc-1".into(),
            "shell::exec".into(),
            json!({"command": "ls"}),
            "sess_a".into(),
            1_000,
            60_000,
        )
    }

    #[test]
    fn pending_has_no_outcome_and_no_resolved_at() {
        let r = pending_record();
        assert_eq!(r.status, Status::Pending);
        assert!(r.outcome.is_none());
        assert!(r.resolved_at.is_none());
        assert_eq!(r.expires_at, 61_000);
    }

    #[test]
    fn pending_expires_at_saturates_on_overflow() {
        let r = Record::pending(
            "tc-1".into(),
            "f".into(),
            json!({}),
            "s".into(),
            u64::MAX - 5,
            100,
        );
        assert_eq!(r.expires_at, u64::MAX);
    }

    #[test]
    fn in_flight_preserves_fields_and_clears_outcome_state() {
        let p = pending_record();
        let i = p.clone().in_flight(2_000);
        assert_eq!(i.status, Status::InFlight);
        assert_eq!(i.function_call_id, p.function_call_id);
        assert_eq!(i.session_id, p.session_id);
        assert_eq!(i.args, p.args);
        assert!(i.outcome.is_none());
        assert_eq!(i.resolved_at, Some(2_000), "InFlight stamps resolved_at");
    }

    #[test]
    fn done_stamps_outcome_and_preserves_in_flight_resolved_at() {
        let i = pending_record().in_flight(2_000);
        let d = i.clone().done(Outcome::Executed {
            result: json!({"ok": true}),
        });
        assert_eq!(d.status, Status::Done);
        assert!(matches!(d.outcome, Some(Outcome::Executed { .. })));
        // resolved_at was set at InFlight time and must NOT be re-stamped on Done.
        assert_eq!(d.resolved_at, Some(2_000));
    }

    #[test]
    fn done_directly_from_pending_stamps_resolved_at() {
        // Deny path skips InFlight; we still need a resolved_at for ordering.
        let p = pending_record();
        let d = p.done_at(
            3_000,
            Outcome::Denied {
                denial: Denial::UserRejected,
            },
        );
        assert_eq!(d.status, Status::Done);
        assert_eq!(d.resolved_at, Some(3_000));
    }

    #[test]
    fn outcome_round_trip_via_json() {
        for o in [
            Outcome::Executed {
                result: json!({"x": 1}),
            },
            Outcome::Failed {
                error: "boom".into(),
            },
            Outcome::Denied {
                denial: Denial::UserRejected,
            },
            Outcome::TimedOut,
        ] {
            let v = serde_json::to_value(&o).unwrap();
            let back: Outcome = serde_json::from_value(v).unwrap();
            // Exhaustive equality is verbose; just round-trip the discriminant.
            assert_eq!(std::mem::discriminant(&o), std::mem::discriminant(&back));
        }
    }

    #[test]
    fn record_round_trip_pending() {
        let r = pending_record();
        let v = r.to_value();
        let back = Record::from_value(v).expect("deserialize");
        assert_eq!(back.status, Status::Pending);
        assert_eq!(back.function_call_id, "tc-1");
    }

    #[test]
    fn record_round_trip_done_carries_outcome_and_resolved_at() {
        let r = pending_record().in_flight(2_000).done(Outcome::Executed {
            result: json!({"out": "hi"}),
        });
        let v = r.to_value();
        let back = Record::from_value(v).expect("deserialize");
        assert_eq!(back.status, Status::Done);
        assert_eq!(back.resolved_at, Some(2_000));
        assert!(matches!(back.outcome, Some(Outcome::Executed { .. })));
    }

    #[test]
    fn flip_returns_none_when_not_expired() {
        let r = pending_record();
        assert!(r.flipped_to_timed_out_if_expired(60_000).is_none());
    }

    #[test]
    fn flip_returns_done_timed_out_for_expired_pending() {
        let r = pending_record();
        let flipped = r
            .flipped_to_timed_out_if_expired(70_000)
            .expect("expired pending should flip");
        assert_eq!(flipped.status, Status::Done);
        assert!(matches!(flipped.outcome, Some(Outcome::TimedOut)));
        assert_eq!(flipped.resolved_at, Some(70_000));
    }

    #[test]
    fn flip_does_not_touch_in_flight_rows_inside_grace_window() {
        let r = pending_record().in_flight(2_000);
        assert!(
            r.flipped_to_timed_out_if_expired(70_000).is_none(),
            "InFlight rows inside the grace window are owned by an in-progress \
             invoke; lazy flip must not steal them"
        );
    }

    #[test]
    fn flip_reclaims_in_flight_rows_past_grace_window() {
        let r = pending_record().in_flight(2_000);
        // Well past resolved_at + IN_FLIGHT_GRACE_MS — the invoke has
        // either wedged or its Done write was lost (see finding #5).
        let flipped = r
            .flipped_to_timed_out_if_expired(u64::MAX)
            .expect("stale InFlight row must reclaim past grace");
        assert_eq!(flipped.status, Status::Done);
        assert!(matches!(flipped.outcome, Some(Outcome::TimedOut)));
    }

    #[test]
    fn flip_does_not_touch_already_done_rows() {
        let r = pending_record()
            .in_flight(2_000)
            .done(Outcome::Executed { result: json!({}) });
        assert!(r.flipped_to_timed_out_if_expired(70_000).is_none());
    }
}
