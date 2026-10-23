//! Deadline timers that sleep until a known due time instead of waking on a
//! cadence to re-check. The owner names the due time and a [`Notify`] that
//! fires whenever that time may have moved (activity, a watcher leaving, a
//! setting changing); between those events and the deadline nothing wakes.

use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};

/// Return once `due()` has passed. `due` is re-read only when `changed` is
/// notified or the deadline it last named arrives (it may have moved later
/// meanwhile, in which case the timer re-arms on the new one). `None` means
/// nothing is due right now (disabled, or held off): only a change can make
/// it due again.
///
/// `changed` is meant for `notify_one`: a notification sent while this is
/// between waits is kept as a permit, so no change is ever lost.
pub async fn until_due(changed: &Notify, mut due: impl FnMut() -> Option<Instant>) {
    loop {
        match due() {
            None => changed.notified().await,
            Some(at) if at <= Instant::now() => return,
            Some(at) => {
                tokio::select! {
                    _ = changed.notified() => {}
                    _ = sleep_until(at) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    /// A tab-like deadline: `last_used + idle`, held off while `held`.
    struct Clock {
        last_used: Mutex<Instant>,
        held: Mutex<bool>,
        idle: Duration,
        reads: AtomicU32,
        changed: Notify,
    }

    impl Clock {
        fn new(idle: Duration) -> Arc<Self> {
            Arc::new(Self {
                last_used: Mutex::new(Instant::now()),
                held: Mutex::new(false),
                idle,
                reads: AtomicU32::new(0),
                changed: Notify::new(),
            })
        }
        fn due(&self) -> Option<Instant> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if *self.held.lock().unwrap() {
                return None;
            }
            Some(*self.last_used.lock().unwrap() + self.idle)
        }
        fn touch(&self) {
            *self.last_used.lock().unwrap() = Instant::now();
            self.changed.notify_one();
        }
        fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<Instant> {
            let clock = self.clone();
            tokio::spawn(async move {
                until_due(&clock.changed, || clock.due()).await;
                Instant::now()
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fires_at_the_deadline_without_waking_before_it() {
        let start = Instant::now();
        let clock = Clock::new(Duration::from_secs(30 * 60));
        let fired = clock.spawn().await.unwrap();
        assert_eq!(fired - start, Duration::from_secs(30 * 60));
        // One read to arm, one when the deadline arrived: nothing in between.
        assert_eq!(clock.reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn activity_rearms_the_deadline() {
        let start = Instant::now();
        let clock = Clock::new(Duration::from_secs(60));
        let timer = clock.spawn();
        tokio::time::sleep(Duration::from_secs(45)).await;
        clock.touch();
        tokio::time::sleep(Duration::from_secs(45)).await;
        assert!(
            !timer.is_finished(),
            "fired 60s after start despite the touch at 45s"
        );
        let fired = timer.await.unwrap();
        assert_eq!(fired - start, Duration::from_secs(45 + 60));
        // Arm, re-arm on the touch, fire.
        assert_eq!(clock.reads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn held_off_waits_for_a_change_only() {
        let start = Instant::now();
        let clock = Clock::new(Duration::from_secs(60));
        *clock.held.lock().unwrap() = true;
        let timer = clock.spawn();
        // Hours pass while held (a viewer watching): nothing wakes.
        tokio::time::sleep(Duration::from_secs(4 * 3600)).await;
        assert!(!timer.is_finished());
        assert_eq!(clock.reads.load(Ordering::SeqCst), 1);
        // The viewer leaves: the clock starts from the release.
        *clock.held.lock().unwrap() = false;
        clock.touch();
        let fired = timer.await.unwrap();
        assert_eq!(fired - start, Duration::from_secs(4 * 3600 + 60));
    }
}
