//! Rate-limited deprecation warnings for the pubsub worker (MOT-3619, Phase 1).
//!
//! The `publish` function, the `subscribe` trigger type and the `pubsub`
//! configuration entry are deprecated. Behavior is unchanged: this module only
//! logs. It never alters a response, an error or a delivered payload.
//!
//! Rules (mirroring the engine's iii-stream warner):
//! - One warning per `(entry point, caller)` key per [`WARN_INTERVAL`].
//! - At most [`MAX_KEYS`] distinct keys are tracked. When the set is full (after
//!   pruning keys whose interval has elapsed), new callers share the fallback
//!   caller key [`FALLBACK_CALLER`] for that entry point, so memory stays bounded.
//! - Warnings carry only the entry point, the caller worker id and the guide
//!   URL. They NEVER contain payloads, `data`, topic names or subscriber config.
//! - `subscribe` warns on trigger registration only, never per delivered event.
//! - Worker startup warns once per process (not again on configuration reload).
//!
//! Exemption: topic [`STREAM_BRIDGE_TOPIC`] (`stream.events`) never warns. It is
//! the engine's own iii-stream `bridge` adapter
//! (engine/src/workers/stream/adapters/bridge.rs, SDK client name
//! `iii-stream-bridge`), which calls `publish` and binds `subscribe` on that
//! topic for every bridged stream mutation. That is engine-internal traffic, not
//! user code using pubsub; it is covered by the iii-stream deprecation instead.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::{PUBLISH_FUNCTION_ID, TRIGGER_TYPE};

/// Public migration guide for iii-stream and pubsub.
pub const MIGRATION_GUIDE_URL: &str = "https://iii.dev/docs/upgrading/migrate-from-streams";
/// Minimum time between two warnings for the same `(entry point, caller)` key.
pub const WARN_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Maximum number of distinct `(entry point, caller)` keys tracked.
pub const MAX_KEYS: usize = 1024;
/// Caller key shared by every new caller once [`MAX_KEYS`] is reached.
pub const FALLBACK_CALLER: &str = "*";
/// Caller key used when the invocation carries no `_caller_worker_id`.
pub const UNKNOWN_CALLER: &str = "unknown";
/// `tracing` target of every deprecation line.
pub const LOG_TARGET: &str = "iii::deprecation";
/// Entry point name used for the once-per-process startup warning (the
/// worker and its `pubsub` configuration entry).
pub const WORKER_ENTRY: &str = "pubsub";
/// Topic used by the engine's internal iii-stream bridge adapter. Exempt from
/// pubsub deprecation warnings (see the module docs).
pub const STREAM_BRIDGE_TOPIC: &str = "stream.events";
/// Longest caller id kept as a rate-limit key (worker ids are 36-char UUIDs).
const MAX_CALLER_LEN: usize = 64;

/// The standard deprecation sentence for `entry`.
pub fn deprecation_message(entry: &str) -> String {
    format!(
        "{entry} is deprecated (pubsub) and will be removed in a future release (version TBD). \
         Behavior is unchanged for now. Migration guide: {MIGRATION_GUIDE_URL}"
    )
}

/// `true` for topics whose traffic must never produce a pubsub deprecation
/// warning (the engine's iii-stream bridge, see the module docs).
pub fn is_exempt_topic(topic: &str) -> bool {
    topic == STREAM_BRIDGE_TOPIC
}

/// Normalize a caller id: missing or blank -> [`UNKNOWN_CALLER`]; longer than
/// [`MAX_CALLER_LEN`] characters -> truncated, so keys stay small.
pub fn caller_key(caller: Option<&str>) -> String {
    match caller.map(str::trim) {
        Some(c) if !c.is_empty() => c.chars().take(MAX_CALLER_LEN).collect(),
        _ => UNKNOWN_CALLER.to_string(),
    }
}

/// Per-`(entry point, caller)` rate limiter with a bounded key set.
pub struct RateLimiter {
    interval: Duration,
    max_keys: usize,
    last_warned: Mutex<HashMap<(String, String), Instant>>,
}

impl RateLimiter {
    pub fn new(interval: Duration, max_keys: usize) -> Self {
        Self {
            interval,
            max_keys,
            last_warned: Mutex::new(HashMap::new()),
        }
    }

    /// Decide whether `(entry, caller)` should warn at `now`. Returns the caller
    /// key the warning was recorded under (`caller` itself, or
    /// [`FALLBACK_CALLER`] once the key set is full), or `None` when suppressed.
    ///
    /// The map holds at most `max_keys` caller keys plus one fallback key per
    /// entry point.
    pub fn check(&self, entry: &str, caller: &str, now: Instant) -> Option<String> {
        let mut last = self
            .last_warned
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let mut key = (entry.to_string(), caller.to_string());
        if !last.contains_key(&key) && last.len() >= self.max_keys {
            // Expired keys would warn again anyway: dropping them is lossless.
            let interval = self.interval;
            last.retain(|_, at| now.saturating_duration_since(*at) < interval);
            if last.len() >= self.max_keys {
                key.1 = FALLBACK_CALLER.to_string();
            }
        }

        match last.get(&key) {
            Some(at) if now.saturating_duration_since(*at) < self.interval => None,
            _ => {
                last.insert(key.clone(), now);
                Some(key.1)
            }
        }
    }

    /// Forget every recorded key (test hook).
    pub fn reset(&self) {
        self.last_warned
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

fn global() -> &'static RateLimiter {
    static LIMITER: OnceLock<RateLimiter> = OnceLock::new();
    LIMITER.get_or_init(|| RateLimiter::new(WARN_INTERVAL, MAX_KEYS))
}

/// Reset the process-wide limiter (test hook).
pub fn reset_for_tests() {
    global().reset();
}

/// Rate-limited warning for `entry` called by `caller`. Returns `true` when a
/// line was emitted.
fn warn_entry(limiter: &RateLimiter, entry: &str, caller: Option<&str>, now: Instant) -> bool {
    let caller = caller_key(caller);
    let Some(rate_key) = limiter.check(entry, &caller, now) else {
        return false;
    };
    tracing::warn!(
        target: LOG_TARGET,
        entry_point = entry,
        caller_worker_id = %caller,
        rate_limit_key = %rate_key,
        guide = MIGRATION_GUIDE_URL,
        "{}",
        deprecation_message(entry)
    );
    true
}

fn warn_publish_with(
    limiter: &RateLimiter,
    topic: &str,
    caller: Option<&str>,
    now: Instant,
) -> bool {
    if is_exempt_topic(topic) {
        return false;
    }
    warn_entry(limiter, PUBLISH_FUNCTION_ID, caller, now)
}

fn warn_subscribe_with(
    limiter: &RateLimiter,
    topic: &str,
    caller: Option<&str>,
    now: Instant,
) -> bool {
    if is_exempt_topic(topic) {
        return false;
    }
    warn_entry(limiter, TRIGGER_TYPE, caller, now)
}

/// Warn (rate-limited) that `publish` is deprecated. `topic` is used only to
/// apply the stream-bridge exemption; it is never logged. `caller` is the
/// invocation's `_caller_worker_id`, if any.
pub fn warn_publish(topic: &str, caller: Option<&str>) -> bool {
    warn_publish_with(global(), topic, caller, Instant::now())
}

/// Warn (rate-limited) that the `subscribe` trigger type is deprecated. Call
/// on trigger registration only. `topic` is used only for the stream-bridge
/// exemption and is never logged. Trigger providers receive no caller worker
/// id from the engine, so callers normally pass `None` (`"unknown"`).
pub fn warn_subscribe(topic: &str, caller: Option<&str>) -> bool {
    warn_subscribe_with(global(), topic, caller, Instant::now())
}

/// Warn once per process that the pubsub worker is deprecated.
pub fn warn_worker_startup() -> bool {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if WARNED.swap(true, Ordering::SeqCst) {
        return false;
    }
    tracing::warn!(
        target: LOG_TARGET,
        entry_point = WORKER_ENTRY,
        functions = PUBLISH_FUNCTION_ID,
        trigger_types = TRIGGER_TYPE,
        guide = MIGRATION_GUIDE_URL,
        "{}",
        deprecation_message(WORKER_ENTRY)
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    fn limiter(max_keys: usize) -> RateLimiter {
        RateLimiter::new(WARN_INTERVAL, max_keys)
    }

    #[test]
    fn message_is_the_standard_sentence() {
        assert_eq!(
            deprecation_message("publish"),
            "publish is deprecated (pubsub) and will be removed in a future release (version TBD). \
             Behavior is unchanged for now. Migration guide: \
             https://iii.dev/docs/upgrading/migrate-from-streams"
        );
    }

    #[test]
    fn suppresses_repeats_within_the_interval_and_rewarns_after() {
        let l = limiter(MAX_KEYS);
        let t0 = Instant::now();
        assert!(warn_publish_with(&l, "orders", Some("w1"), t0));
        assert!(!warn_publish_with(&l, "orders", Some("w1"), t0));
        // Topic is not part of the key: another topic is still suppressed.
        assert!(!warn_publish_with(
            &l,
            "invoices",
            Some("w1"),
            t0 + Duration::from_secs(60)
        ));
        assert!(!warn_publish_with(
            &l,
            "orders",
            Some("w1"),
            t0 + WARN_INTERVAL - Duration::from_millis(1)
        ));
        assert!(warn_publish_with(
            &l,
            "orders",
            Some("w1"),
            t0 + WARN_INTERVAL
        ));
    }

    #[test]
    fn distinct_callers_and_entries_warn_independently() {
        let l = limiter(MAX_KEYS);
        let t0 = Instant::now();
        assert!(warn_publish_with(&l, "orders", Some("w1"), t0));
        assert!(warn_publish_with(&l, "orders", Some("w2"), t0));
        assert!(warn_publish_with(&l, "orders", None, t0));
        assert!(
            !warn_publish_with(&l, "orders", Some(""), t0),
            "blank == unknown"
        );
        assert!(warn_subscribe_with(&l, "orders", Some("w1"), t0));
        assert!(!warn_subscribe_with(&l, "orders", Some("w1"), t0));
    }

    #[test]
    fn full_key_set_falls_back_to_star() {
        let l = limiter(2);
        let t0 = Instant::now();
        assert_eq!(l.check("publish", "a", t0), Some("a".to_string()));
        assert_eq!(l.check("publish", "b", t0), Some("b".to_string()));
        // Set full: a new caller warns once under the shared fallback key...
        assert_eq!(l.check("publish", "c", t0), Some("*".to_string()));
        // ...and further new callers are suppressed by it.
        assert_eq!(l.check("publish", "d", t0), None);
        // Known callers keep their own key.
        assert_eq!(l.check("publish", "a", t0), None);
        assert_eq!(l.last_warned.lock().unwrap().len(), 3, "cap + one fallback");

        // Once the interval elapses, expired keys are pruned and new callers
        // get their own key again.
        let later = t0 + WARN_INTERVAL;
        assert_eq!(l.check("publish", "e", later), Some("e".to_string()));
        assert!(l.last_warned.lock().unwrap().len() <= 3);
    }

    #[test]
    fn reset_forgets_keys() {
        let l = limiter(MAX_KEYS);
        let t0 = Instant::now();
        assert!(l.check("publish", "a", t0).is_some());
        l.reset();
        assert!(l.check("publish", "a", t0).is_some());
    }

    #[test]
    fn stream_bridge_topic_is_exempt() {
        let l = limiter(MAX_KEYS);
        let t0 = Instant::now();
        for caller in [None, Some("bridge-worker")] {
            assert!(!warn_publish_with(&l, STREAM_BRIDGE_TOPIC, caller, t0));
            assert!(!warn_subscribe_with(&l, STREAM_BRIDGE_TOPIC, caller, t0));
        }
        assert!(l.last_warned.lock().unwrap().is_empty());
        // The global entry points honor the exemption too.
        assert!(!warn_publish("stream.events", Some("bridge-worker")));
        assert!(!warn_subscribe("stream.events", None));
        // Exact match only.
        assert!(warn_publish_with(&l, "stream.events.other", None, t0));
    }

    #[test]
    fn caller_key_normalizes() {
        assert_eq!(caller_key(None), UNKNOWN_CALLER);
        assert_eq!(caller_key(Some("   ")), UNKNOWN_CALLER);
        assert_eq!(caller_key(Some("abc")), "abc");
        assert_eq!(caller_key(Some(&"x".repeat(500))).len(), MAX_CALLER_LEN);
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Equivalent of the live check: two publishes to a normal topic produce
    /// exactly one deprecation line; `stream.events` produces none; the line
    /// carries no topic and no payload.
    #[test]
    fn emits_one_line_per_key_without_topic_or_payload() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        let l = limiter(MAX_KEYS);
        let t0 = Instant::now();
        tracing::subscriber::with_default(subscriber, || {
            warn_publish_with(&l, "secret-topic", Some("worker-1"), t0);
            warn_publish_with(&l, "secret-topic", Some("worker-1"), t0);
            warn_publish_with(&l, STREAM_BRIDGE_TOPIC, Some("worker-1"), t0);
            warn_subscribe_with(&l, STREAM_BRIDGE_TOPIC, None, t0);
        });
        let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 1, "{out}");
        let line = lines[0];
        assert!(line.contains("WARN"));
        assert!(line.contains(LOG_TARGET));
        assert!(line.contains(&deprecation_message("publish")));
        assert!(line.contains("entry_point=\"publish\""));
        assert!(line.contains("caller_worker_id=worker-1"));
        assert!(!line.contains("secret-topic"));
        assert!(!line.contains(STREAM_BRIDGE_TOPIC));
    }

    #[test]
    fn startup_warns_once_per_process() {
        let first = warn_worker_startup();
        assert!(!warn_worker_startup());
        // Another test in this process may have triggered it first; either
        // way, at most one call ever returns true.
        let _ = first;
    }
}
