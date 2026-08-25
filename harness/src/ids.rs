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

/// The opening task entry of a direct (agent-called) spawn (`e_spawn_<uuid>`).
/// Same pattern as `e_spawned_`: the prefix survives transcript reads so the
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

/// A names-only skill correction for one model generation. Redelivery of the
/// same generation appends to the same transcript entry.
pub fn skill_update_entry_id(turn_id: &str, generation: u32) -> String {
    format!("e_{turn_id}_skills_{generation}")
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

/// Keep ids filesystem/key safe without collapsing distinct external values.
/// Safe ASCII bytes remain readable and every other UTF-8 byte is escaped as
/// `~HH`. `~` is always escaped too, making the representation reversible.
fn sanitize(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('~');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
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
        assert_eq!(skill_update_entry_id("t_1", 4), "e_t_1_skills_4");
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
    fn sanitize_escapes_unsafe_chars_without_ambiguity() {
        assert_eq!(function_result_entry_id("t_1", "a/b c"), "e_t_1_a~2Fb~20c");
        assert_ne!(sanitize("a/"), sanitize("a~2F"));
        assert_ne!(sanitize("é"), sanitize("__"));
    }

    #[test]
    fn distinct_external_ids_never_collapse_to_one_entry() {
        assert_ne!(idem_user_entry_id("idem/a"), idem_user_entry_id("idem a"));
        assert_ne!(
            function_result_entry_id("t_1", "call/a"),
            function_result_entry_id("t_1", "call a")
        );
    }

    #[test]
    fn fresh_ids_are_prefixed_and_unique() {
        assert!(new_session_id().starts_with("s_"));
        assert!(new_turn_id().starts_with("t_"));
        assert_ne!(new_turn_id(), new_turn_id());
    }
}
