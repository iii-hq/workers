//! A provider's model, loaded on first use and released when idle.
//!
//! The judge hub routes a request to any `judge-<provider>`, so a local
//! provider answers even when it is not the hub's default: the first call
//! starts one shared load and waits for it within its own deadline, and the
//! load goes on for later calls if that deadline passes. A pinned slot (the
//! hub's default, or `preload_all`) loads right away and is never released.
use judge_contract::{ErrorCode, EvaluateResponse, ModelsResponse, ProviderError, Stats};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

/// An unpinned model is released after this long without a call.
// ponytail: fixed; make it a provider config field if operators need to tune it.
pub const IDLE_RELEASE: Duration = Duration::from_secs(600);
/// A failed load answers with its error for this long before a call retries it,
/// so a missing checkpoint or a full GPU is not reloaded on every call.
const RETRY_LOAD_AFTER: Duration = Duration::from_secs(30);

/// Why a judge call cannot reach the model, as the contract's error fields:
/// `deadline` while the load runs (it goes on), `provider_unavailable` once it
/// failed (a call after `retry_after_ms` loads again).
#[derive(Clone, Debug)]
pub struct Unloaded {
    pub code: ErrorCode,
    pub provider_error: Option<ProviderError>,
    pub retry_after_ms: Option<u64>,
}
impl Unloaded {
    fn deadline() -> Self {
        Self {
            code: ErrorCode::Deadline,
            provider_error: None,
            retry_after_ms: None,
        }
    }
    fn failed(message: &str, retry_after: Duration) -> Self {
        Self {
            code: ErrorCode::ProviderUnavailable,
            provider_error: Some(ProviderError {
                detail: None,
                message: Some(format!("the model failed to load: {message}")),
                truncated: false,
            }),
            retry_after_ms: Some(retry_after.as_millis() as u64),
        }
    }
    pub fn evaluate_error(self) -> EvaluateResponse {
        EvaluateResponse::Error {
            code: self.code,
            http_status: None,
            provider_error: self.provider_error,
            retry_after_ms: self.retry_after_ms,
            stats: Stats::default(),
        }
    }
    pub fn models_error(self) -> ModelsResponse {
        ModelsResponse::Error {
            code: self.code,
            http_status: None,
            provider_error: self.provider_error,
            retry_after_ms: self.retry_after_ms,
            stats: Stats::default(),
        }
    }
}

type Load<T> = Arc<dyn Fn() -> anyhow::Result<T> + Send + Sync>;

pub struct ModelSlot<T> {
    load: Load<T>,
    state: Mutex<State<T>>,
}

struct State<T> {
    model: Model<T>,
    pinned: bool,
    last_used: Instant,
}

enum Model<T> {
    Empty,
    /// Flips to true once the load stored its outcome.
    Loading(watch::Receiver<bool>),
    Ready(T),
    Failed {
        message: String,
        at: Instant,
    },
}

impl<T: Clone + Send + 'static> ModelSlot<T> {
    /// `load` runs on a blocking thread each time the model is (re)loaded.
    pub fn new(load: impl Fn() -> anyhow::Result<T> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            load: Arc::new(load),
            state: Mutex::new(State {
                model: Model::Empty,
                pinned: false,
                last_used: Instant::now(),
            }),
        })
    }

    /// The model if it is loaded now; never starts a load.
    pub fn loaded(&self) -> Option<T> {
        match &self.lock().model {
            Model::Ready(model) => Some(model.clone()),
            _ => None,
        }
    }

    /// The model, loading it if needed and waiting at most until `deadline`.
    pub async fn get(self: &Arc<Self>, deadline: Instant) -> Result<T, Unloaded> {
        loop {
            let mut done = {
                let mut state = self.lock();
                state.last_used = Instant::now();
                match &state.model {
                    Model::Ready(model) => return Ok(model.clone()),
                    Model::Failed { message, at } if at.elapsed() < RETRY_LOAD_AFTER => {
                        return Err(Unloaded::failed(message, RETRY_LOAD_AFTER - at.elapsed()))
                    }
                    Model::Loading(done) => done.clone(),
                    Model::Empty | Model::Failed { .. } => self.start(&mut state),
                }
            };
            let stored = tokio::time::timeout_at(deadline.into(), done.wait_for(|done| *done))
                .await
                .map(|stored| stored.is_ok());
            match stored {
                // Stored: Ready or Failed on the next pass.
                Ok(true) => continue,
                // Only when the runtime shuts down mid-load.
                Ok(false) => {
                    return Err(Unloaded::failed(
                        "the load ended without a result",
                        RETRY_LOAD_AFTER,
                    ))
                }
                Err(_) => return Err(Unloaded::deadline()),
            }
        }
    }

    /// The model for a judge request, waiting for its load at most the
    /// request's own budget (`timeout_ms`, capped by `expires_at_unix_ms`),
    /// and taking that wait from `timeout_ms`.
    pub async fn for_request(
        self: &Arc<Self>,
        timeout_ms: &mut u64,
        expires_at_unix_ms: Option<u64>,
    ) -> Result<T, Unloaded> {
        let started = Instant::now();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let budget =
            expires_at_unix_ms.map_or(*timeout_ms, |at| at.saturating_sub(now_ms).min(*timeout_ms));
        let model = self.get(started + Duration::from_millis(budget)).await?;
        let waited = started.elapsed().as_millis() as u64;
        if waited > 0 {
            *timeout_ms = timeout_ms.saturating_sub(waited);
            if *timeout_ms == 0 {
                return Err(Unloaded::deadline());
            }
        }
        Ok(model)
    }

    /// Pinned: load now and never release. Unpinned: release after `idle`
    /// without a call (see [`Self::release_idle`]).
    pub fn pin(self: &Arc<Self>, pinned: bool) {
        let mut state = self.lock();
        state.pinned = pinned;
        // The idle clock starts when the hub stops selecting this provider.
        state.last_used = Instant::now();
        if pinned && matches!(state.model, Model::Empty | Model::Failed { .. }) {
            self.start(&mut state);
        }
    }

    /// Pin the model while the judge hub's configuration (`hub`, as
    /// `iii_config_client::follow` yields it) makes `provider` its default or
    /// preloads every provider; otherwise it loads on demand. Runs until the
    /// hub's watch closes.
    pub async fn follow_selection(
        self: &Arc<Self>,
        mut hub: watch::Receiver<Option<Value>>,
        provider: &str,
    ) -> anyhow::Result<()> {
        let mut pinned = None;
        loop {
            let selects = hub.borrow().as_ref().is_some_and(|hub| {
                hub.get("provider").and_then(Value::as_str) == Some(provider)
                    || hub.get("preload_all").and_then(Value::as_bool) == Some(true)
            });
            if pinned != Some(selects) {
                tracing::info!(provider, pinned = selects, "judge hub selection changed");
                self.pin(selects);
                pinned = Some(selects);
            }
            hub.changed().await?;
        }
    }

    /// Release an unpinned model after `idle` without a call, and retry a
    /// pinned model's failed load. In-flight calls keep their own handle, so
    /// the model is freed once the last of them ends. Runs until dropped.
    pub async fn release_idle(self: Arc<Self>, idle: Duration) {
        let mut tick = tokio::time::interval((idle / 4).max(Duration::from_millis(1)));
        loop {
            tick.tick().await;
            let mut state = self.lock();
            let (release, retry) = match &state.model {
                Model::Ready(_) => (!state.pinned && state.last_used.elapsed() >= idle, false),
                Model::Failed { at, .. } => {
                    (false, state.pinned && at.elapsed() >= RETRY_LOAD_AFTER)
                }
                _ => (false, false),
            };
            if release {
                state.model = Model::Empty;
                tracing::info!(idle_secs = idle.as_secs(), "model idle; released");
            }
            if retry {
                self.start(&mut state);
            }
        }
    }

    fn start(self: &Arc<Self>, state: &mut State<T>) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        state.model = Model::Loading(rx.clone());
        let slot = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            tracing::info!("loading the model");
            let load = slot.load.clone();
            let outcome = match tokio::task::spawn_blocking(move || load()).await {
                Ok(loaded) => loaded.map_err(|error| format!("{error:#}")),
                Err(error) => Err(format!("the model load panicked: {error}")),
            };
            let model = match outcome {
                Ok(model) => {
                    tracing::info!(
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "model loaded"
                    );
                    Model::Ready(model)
                }
                Err(message) => {
                    tracing::warn!(error = %message, "model load failed");
                    Model::Failed {
                        message,
                        at: Instant::now(),
                    }
                }
            };
            slot.lock().model = model;
            let _ = tx.send(true);
        });
        rx
    }

    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().expect("model slot lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting(delay_ms: u64, ok: bool) -> (Arc<ModelSlot<usize>>, Arc<AtomicUsize>) {
        let loads = Arc::new(AtomicUsize::new(0));
        let counter = loads.clone();
        let slot = ModelSlot::new(move || {
            std::thread::sleep(Duration::from_millis(delay_ms));
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if ok {
                Ok(n)
            } else {
                anyhow::bail!("no checkpoint")
            }
        });
        (slot, loads)
    }

    fn within(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[tokio::test]
    async fn concurrent_calls_share_one_load() {
        let (slot, loads) = counting(50, true);
        let calls: Vec<_> = (0..5)
            .map(|_| {
                let slot = slot.clone();
                tokio::spawn(async move { slot.get(within(2_000)).await })
            })
            .collect();
        for call in calls {
            assert_eq!(call.await.unwrap().unwrap(), 1);
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_passed_deadline_leaves_the_load_running() {
        let (slot, loads) = counting(100, true);
        let unloaded = slot.get(within(10)).await.unwrap_err();
        assert!(matches!(unloaded.code, ErrorCode::Deadline));
        assert_eq!(slot.loaded(), None);
        assert_eq!(slot.get(within(2_000)).await.unwrap(), 1);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_load_is_not_retried_at_once() {
        let (slot, loads) = counting(0, false);
        for _ in 0..3 {
            let unloaded = slot.get(within(2_000)).await.unwrap_err();
            assert!(matches!(unloaded.code, ErrorCode::ProviderUnavailable));
            let message = unloaded.provider_error.and_then(|error| error.message);
            assert_eq!(
                message.as_deref(),
                Some("the model failed to load: no checkpoint")
            );
            assert!(unloaded.retry_after_ms <= Some(RETRY_LOAD_AFTER.as_millis() as u64));
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_request_waits_within_its_own_budget() {
        let (slot, _) = counting(100, true);
        let mut timeout_ms = 20;
        let unloaded = slot.for_request(&mut timeout_ms, None).await.unwrap_err();
        assert!(matches!(unloaded.code, ErrorCode::Deadline));
        let mut timeout_ms = 5_000;
        assert_eq!(slot.for_request(&mut timeout_ms, None).await.unwrap(), 1);
        assert!(
            timeout_ms < 5_000,
            "the wait comes off the request's budget"
        );
        let (failing, _) = counting(0, false);
        let unloaded = failing.for_request(&mut 5_000, None).await.unwrap_err();
        assert!(matches!(unloaded.code, ErrorCode::ProviderUnavailable));
        assert!(unloaded.retry_after_ms.is_some());
    }

    #[tokio::test]
    async fn only_an_unpinned_idle_model_is_released() {
        let (unpinned, _) = counting(0, true);
        assert_eq!(unpinned.get(within(2_000)).await.unwrap(), 1);
        let (pinned, _) = counting(0, true);
        pinned.pin(true);
        assert_eq!(pinned.get(within(2_000)).await.unwrap(), 1);
        tokio::spawn(unpinned.clone().release_idle(Duration::from_millis(40)));
        tokio::spawn(pinned.clone().release_idle(Duration::from_millis(40)));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(unpinned.loaded(), None);
        assert_eq!(pinned.loaded(), Some(1));
        // Unpinned, the pinned one is released too; a call loads it again.
        pinned.pin(false);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(pinned.loaded(), None);
        assert_eq!(pinned.get(within(2_000)).await.unwrap(), 2);
    }
}
