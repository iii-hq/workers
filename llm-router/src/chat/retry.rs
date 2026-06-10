//! Backoff schedule. What retries (rate_limited|transient via
//! ErrorKind::is_retryable) and when (pre-first-frame only) is enforced in
//! chat.rs (spec § Retries).

/// Capped exponential with half-jitter: step = base * 2^attempt,
/// delay ∈ [step/2, step] — so base_ms is the floor of the first retry delay.
pub fn backoff_ms(attempt: u32, base_ms: u64, cap_ms: u64, random: impl Fn() -> f64) -> u64 {
    let step = (base_ms.saturating_mul(2u64.saturating_pow(attempt))).min(cap_ms);
    ((step as f64) * (0.5 + random() / 2.0)).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_is_capped_exponential_with_half_jitter() {
        assert_eq!(backoff_ms(1, 500, 8000, || 1.0), 1000);
        assert_eq!(backoff_ms(1, 500, 8000, || 0.0), 500);
        assert_eq!(backoff_ms(2, 500, 8000, || 1.0), 2000);
        assert_eq!(backoff_ms(10, 500, 8000, || 1.0), 8000); // cap
    }
}
