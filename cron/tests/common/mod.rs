pub mod engine;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Waits until the counter reaches `n` (up to about 6s).
pub async fn wait_for_fires(counter: &Arc<AtomicUsize>, n: usize) {
    for _ in 0..60 {
        if counter.load(Ordering::SeqCst) >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("cron job never reached {n} fire(s)");
}
