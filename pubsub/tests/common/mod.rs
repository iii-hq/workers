pub mod engine;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Waits until the counter reaches `n` (up to ~5s), else panics.
pub async fn wait_for_deliveries(counter: &Arc<AtomicUsize>, n: usize) {
    for _ in 0..50 {
        if counter.load(Ordering::SeqCst) >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("subscribers never reached {n} deliver(ies)");
}
