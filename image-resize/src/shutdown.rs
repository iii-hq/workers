//! A stop lets the calls in flight finish before the process exits.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// How long a stop waits for calls in flight: under the 10 s iii compose
/// waits by default between its SIGTERM and its SIGKILL.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(8);

/// Counts the `image_resize::resize` calls in flight.
#[derive(Default)]
pub struct InFlight {
    calls: AtomicUsize,
    idle: Notify,
}

/// Counts one call until it is dropped.
pub struct CallGuard(Arc<InFlight>);

impl InFlight {
    pub fn enter(self: &Arc<Self>) -> CallGuard {
        self.calls.fetch_add(1, Ordering::SeqCst);
        CallGuard(Arc::clone(self))
    }

    pub fn running(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Waits until no call is in flight, or `deadline` passes, and returns
    /// the number of calls still running.
    pub async fn drain(&self, deadline: Duration) -> usize {
        let _ = tokio::time::timeout(deadline, async {
            loop {
                // Created before the check, so a guard dropped in between still wakes it.
                let idle = self.idle.notified();
                if self.running() == 0 {
                    return;
                }
                idle.await;
            }
        })
        .await;
        self.running()
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if self.0.calls.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[tokio::test]
    async fn drain_waits_for_the_call_in_flight() {
        let calls = Arc::new(InFlight::default());
        let call = calls.enter();
        let finish = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            drop(call);
        });
        let started = Instant::now();
        assert_eq!(calls.drain(Duration::from_secs(10)).await, 0);
        assert!(started.elapsed() >= Duration::from_millis(200));
        assert!(started.elapsed() < Duration::from_secs(10));
        finish.await.unwrap();
    }

    #[tokio::test]
    async fn drain_returns_at_the_deadline_when_a_call_hangs() {
        let calls = Arc::new(InFlight::default());
        let _hung = calls.enter();
        let started = Instant::now();
        assert_eq!(calls.drain(Duration::from_millis(200)).await, 1);
        assert!(started.elapsed() >= Duration::from_millis(200));
    }

    #[tokio::test]
    async fn drain_with_nothing_in_flight_returns_at_once() {
        let calls = Arc::new(InFlight::default());
        drop(calls.enter());
        let started = Instant::now();
        assert_eq!(calls.drain(Duration::from_secs(10)).await, 0);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
