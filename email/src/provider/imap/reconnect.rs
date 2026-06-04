//! Event-driven reconnect backoff. We sleep ONLY between connect attempts —
//! never as a substitute for IDLE.

use std::time::Duration;

/// Exponential backoff with jitter. attempt=1 → 1s, capped at 60s.
pub async fn backoff(attempt: u32) {
    if attempt == 0 {
        return;
    }
    let base = 2u64.saturating_pow(attempt.min(6) - 1);
    let secs = base.min(60);
    // Tiny pseudo-random jitter to avoid thundering herds without pulling
    // in `rand`. UID-style hash of nanos is good enough here.
    let jitter_ms = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
        % 250) as u64;
    let dur = Duration::from_millis(secs * 1000 + jitter_ms);
    tracing::info!(
        attempt,
        dur_ms = dur.as_millis() as u64,
        "imap reconnect backoff"
    );
    tokio::time::sleep(dur).await;
}
