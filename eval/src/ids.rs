use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Prefix of every investigation session the monitor creates. Recursion
/// guard only — it is not an access check.
pub const ANALYST_PREFIX: &str = "eval_monitor_";
/// Percentage of otherwise quiet sessions sent to the LLM for audit.
pub const AUDIT_SAMPLE_PERCENT: u32 = 5;

pub fn evaluation_id() -> String {
    format!("eval_{}", Uuid::new_v4().simple())
}

/// Stable identity of one observed session turn. The length prefix keeps the
/// two parts unambiguous.
pub fn observation_key(session_id: &str, turn_id: &str) -> String {
    sha256_text(&format!("{}:{session_id}{turn_id}", session_id.len()))
}

pub fn analyst_session(evaluation_id: &str) -> String {
    format!("{ANALYST_PREFIX}{evaluation_id}")
}

pub fn judge_request_id(evaluation_id: &str, step: u64) -> String {
    format!("{evaluation_id}-judge-{step}")
}

/// A fresh id for each E2E-pairing question: the provider may run an id again
/// once its call ended, so a repeated click must not reuse one.
pub fn proposal_request_id(evaluation_id: &str) -> String {
    format!("{evaluation_id}-propose-{}", Uuid::new_v4().simple())
}

/// Deterministic audit sample: the first 32 bits of the observation key,
/// modulo 100.
pub fn audit_sample(observation_key: &str) -> bool {
    observation_key
        .get(..8)
        .and_then(|prefix| u32::from_str_radix(prefix, 16).ok())
        .is_some_and(|value| value % 100 < AUDIT_SAMPLE_PERCENT)
}

pub fn sha256_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn sha256_json(value: &impl serde::Serialize) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_key_is_stable_and_unambiguous() {
        assert_eq!(observation_key("s", "t"), observation_key("s", "t"));
        assert_ne!(observation_key("ab", "c"), observation_key("a", "bc"));
        assert_ne!(evaluation_id(), evaluation_id());
        let first = proposal_request_id("eval_x");
        assert!(first.starts_with("eval_x-propose-") && first.len() <= 128);
        assert_ne!(first, proposal_request_id("eval_x"));
    }

    #[test]
    fn audit_sample_is_about_five_percent() {
        let sampled = (0..10_000)
            .filter(|index| audit_sample(&observation_key("s", &index.to_string())))
            .count();
        assert!((350..650).contains(&sampled), "sampled {sampled}");
    }
}
