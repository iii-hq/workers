//! Trigger policy for compaction. Single function so unit tests and the
//! lib subscriber agree on the decision boundary.

use crate::config::trigger_tokens;

/// Returns `true` when the most recent turn's transcript size has crossed
/// the configured token threshold and compaction should fire.
pub fn should_compact(total_tokens: u64) -> bool {
    total_tokens >= trigger_tokens()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_tokens_never_compacts() {
        assert!(!should_compact(0));
    }

    #[test]
    fn huge_token_count_always_compacts() {
        // 10× the highest sane default — must always cross.
        assert!(should_compact(10_000_000));
    }
}
