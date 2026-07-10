//! Deterministic id derivation (harness.md § Durability & idempotency).
//!
//! Every entry a step writes uses a deterministic id so a redelivered step
//! writes into the SAME entry instead of duplicating it (`session::append`
//! is idempotent on `entry_id`). New sessions/turns get random ids; entries
//! within a turn are derived from `(turn_id, step | function_call_id)`.

use uuid::Uuid;

/// A fresh session id (`s_<uuid>`), used when `harness::send` omits one.
pub fn new_session_id() -> String {
    format!("s_{}", short_uuid())
}

/// A fresh turn id (`t_<uuid>`), minted by the CAS seed.
pub fn new_turn_id() -> String {
    format!("t_{}", short_uuid())
}

/// A fresh function_call id for harness-synthesised calls.
pub fn new_call_id() -> String {
    format!("fc_{}", short_uuid())
}

/// A fresh queued-message id (`q_<uuid>`), minted when a message is enqueued
/// while a step is streaming.
pub fn new_queued_id() -> String {
    format!("q_{}", short_uuid())
}

/// The deterministic entry id a queued message drains into, so a redelivered
/// drain is a no-op.
pub fn queued_entry_id(queued_id: &str) -> String {
    format!("e_{}", sanitize(queued_id))
}

/// The deterministic id of the user entry derived from an idempotency key,
/// so a redelivered webhook append is a no-op.
pub fn idem_user_entry_id(key: &str) -> String {
    format!("e_idem_{}", sanitize(key))
}

/// The opening task entry of a react-fired spawn (`e_react_<uuid>`). The
/// prefix lets transcript reads mark the row as a trigger reaction even
/// though `session::messages` does not return `origin` (the notify pattern).
pub fn react_entry_id() -> String {
    format!("e_react_{}", short_uuid())
}

/// The opening task entry of a direct (agent-called) spawn (`e_spawn_<uuid>`).
/// Same pattern as `e_react_`: the prefix survives transcript reads so the
/// console renders the task as machine-sent, not something the human typed.
pub fn spawn_entry_id() -> String {
    format!("e_spawn_{}", short_uuid())
}

/// The assistant message of a generate step: `e_<turn_id>_<step>_assistant`.
/// A resumed step streams into this same entry rather than appending a second.
pub fn assistant_entry_id(turn_id: &str, step: u64) -> String {
    format!("e_{turn_id}_{step}_assistant")
}

/// A function_result entry: `e_<turn_id>_<function_call_id>`. Used by both a
/// direct trigger and a later `harness::function::resolve` so duplicates hit
/// the same id.
pub fn function_result_entry_id(turn_id: &str, function_call_id: &str) -> String {
    format!("e_{turn_id}_{}", sanitize(function_call_id))
}

/// A compaction custom entry: `e_<turn_id>_<step>_compaction`.
pub fn compaction_entry_id(turn_id: &str, step: u64) -> String {
    format!("e_{turn_id}_{step}_compaction")
}

/// A synthetic validation-nudge user entry for an output-contract retry.
pub fn validation_nudge_entry_id(turn_id: &str, attempt: u32) -> String {
    format!("e_{turn_id}_nudge_{attempt}")
}

/// A synthetic user entry that asks the model to resume after a transient
/// provider/router failure. Distinct from output-validation nudges so both
/// retry loops remain idempotent when they occur in the same turn.
pub fn transient_resume_nudge_entry_id(turn_id: &str, attempt: u32) -> String {
    format!("e_{turn_id}_transient_resume_{attempt}")
}

/// User-visible lifecycle entries for one transient recovery attempt.
pub fn transient_recovery_entry_id(turn_id: &str, attempt: u32, outcome: &str) -> String {
    format!(
        "e_{turn_id}_transient_recovery_{attempt}_{}",
        sanitize(outcome)
    )
}

fn short_uuid() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Keep ids filesystem/key safe: replace anything outside `[A-Za-z0-9_-]`.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_ids_are_stable() {
        assert_eq!(assistant_entry_id("t_1", 3), "e_t_1_3_assistant");
        assert_eq!(function_result_entry_id("t_1", "fc_9"), "e_t_1_fc_9");
        assert_eq!(compaction_entry_id("t_1", 4), "e_t_1_4_compaction");
        assert_eq!(queued_entry_id("q_abc"), "e_q_abc");
        assert_eq!(
            transient_resume_nudge_entry_id("t_1", 2),
            "e_t_1_transient_resume_2"
        );
        assert_eq!(
            transient_recovery_entry_id("t_1", 2, "recovered"),
            "e_t_1_transient_recovery_2_recovered"
        );
    }

    #[test]
    fn sanitize_replaces_unsafe_chars() {
        assert_eq!(function_result_entry_id("t_1", "a/b c"), "e_t_1_a_b_c");
    }

    #[test]
    fn fresh_ids_are_prefixed_and_unique() {
        assert!(new_session_id().starts_with("s_"));
        assert!(new_turn_id().starts_with("t_"));
        assert_ne!(new_turn_id(), new_turn_id());
    }
}
