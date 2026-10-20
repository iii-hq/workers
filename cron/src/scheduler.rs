//! Scheduling loop: one tokio task per distinct cron expression, shared by
//! every `cron` trigger binding and every `cron::on-tick` observer on it.
//! Port of the builtin's structs.rs: UTC only; no catch-up (always the NEXT
//! upcoming fire from now -- missed runs while down are skipped, never
//! replayed).
//!
//! Each fire takes the lock once for its expression, then fans out: every
//! binding is called as the trigger type always did (its own `job_id`,
//! condition and metadata sidecar), every observer gets the [`CronTick`]
//! through [`TickObserver::emit`]. The job stops when its last binding or
//! observer leaves.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use cron::Schedule;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::locks::CronLock;
use crate::on_tick::CronTick;

/// `metadata` is the binding's registration metadata, delivered as the
/// fire-time sidecar (engine `fire_triggers` parity). Handlers like
/// `harness::spawn` / `harness::notify_agent` resolve which reaction or
/// subscription fired from it and silently no-op without it — a plain call
/// here kills every cron-triggered harness reaction and wake-up.
#[async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(
        &self,
        function_id: &str,
        payload: serde_json::Value,
        metadata: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, String>;
}

/// An observer of `cron::on-tick`, as the scheduler sees it: the SDK's
/// `Observer` in the worker (whose `emit` applies the observer's filters and
/// returns `Ok(false)` once it was removed), a recorder in tests.
pub trait TickObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn emit(&self, tick: &CronTick) -> Result<bool, String>;
}

#[derive(Debug, Clone)]
pub struct JobSpec {
    pub trigger_id: String,
    pub expression: String,
    pub function_id: String,
    pub condition_function_id: Option<String>,
    /// Registration metadata, replayed as the fire-time sidecar.
    pub metadata: Option<serde_json::Value>,
}

/// A `cron` trigger binding.
struct Binding {
    spec: JobSpec,
    /// A fire (condition and call) still running. A binding never overlaps
    /// itself: while busy it skips fires, as when each binding had its own
    /// loop.
    in_flight: AtomicBool,
}

/// What a fire fans out to.
#[derive(Clone)]
enum Sink {
    Binding(Arc<Binding>),
    Observer(Arc<dyn TickObserver>),
}

/// Trigger ids and observer ids are both chosen by consumers: kept apart.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SinkKey {
    Trigger(String),
    Observer(String),
}

type Sinks = Arc<std::sync::Mutex<BTreeMap<SinkKey, Sink>>>;

struct JobEntry {
    /// Shared with the job's task, which reads it on every fire.
    sinks: Sinks,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct Jobs {
    by_expression: HashMap<String, JobEntry>,
    expression_of: HashMap<SinkKey, String>,
}

pub struct Scheduler {
    lock: Arc<dyn CronLock>,
    invoker: Arc<dyn Invoker>,
    jobs: Mutex<Jobs>,
}

/// The validation both paths share: a `cron` trigger config and an
/// observer's args are refused with the same message.
pub fn parse_expression(expression: &str) -> anyhow::Result<Schedule> {
    if expression.trim().is_empty() {
        anyhow::bail!("Cron expression is required");
    }
    Schedule::from_str(expression)
        .map_err(|e| anyhow::anyhow!("invalid cron expression '{expression}': {e}"))
}

impl Scheduler {
    pub fn new(lock: Arc<dyn CronLock>, invoker: Arc<dyn Invoker>) -> Self {
        Self {
            lock,
            invoker,
            jobs: Mutex::new(Jobs::default()),
        }
    }

    /// Files a `cron` trigger binding under its expression.
    pub async fn register(&self, spec: JobSpec) -> anyhow::Result<()> {
        let schedule = parse_expression(&spec.expression)?;
        let key = SinkKey::Trigger(spec.trigger_id.clone());
        let mut jobs = self.jobs.lock().await;
        if jobs.expression_of.contains_key(&key) {
            anyhow::bail!("cron trigger '{}' is already registered", spec.trigger_id);
        }
        let expression = spec.expression.clone();
        let sink = Sink::Binding(Arc::new(Binding {
            spec,
            in_flight: AtomicBool::new(false),
        }));
        self.file(&mut jobs, schedule, expression, key, sink);
        Ok(())
    }

    pub async fn unregister(&self, trigger_id: &str) -> anyhow::Result<()> {
        if self.remove(SinkKey::Trigger(trigger_id.to_string())).await {
            Ok(())
        } else {
            anyhow::bail!("cron trigger '{trigger_id}' not found")
        }
    }

    /// Files an observer of `cron::on-tick` under `expression`. An observer
    /// filed again (the engine replayed it) replaces itself.
    pub async fn add_observer(
        &self,
        expression: &str,
        observer: Arc<dyn TickObserver>,
    ) -> anyhow::Result<()> {
        let schedule = parse_expression(expression)?;
        let key = SinkKey::Observer(observer.id().to_string());
        let mut jobs = self.jobs.lock().await;
        if jobs
            .expression_of
            .get(&key)
            .is_some_and(|held| held != expression)
        {
            self.take(&mut jobs, &key).await;
        }
        self.file(
            &mut jobs,
            schedule,
            expression.to_string(),
            key,
            Sink::Observer(observer),
        );
        Ok(())
    }

    pub async fn remove_observer(&self, observer_id: &str) {
        self.remove(SinkKey::Observer(observer_id.to_string()))
            .await;
    }

    /// Joins the job of `expression`, or starts it.
    fn file(
        &self,
        jobs: &mut Jobs,
        schedule: Schedule,
        expression: String,
        key: SinkKey,
        sink: Sink,
    ) {
        jobs.expression_of.insert(key.clone(), expression.clone());
        if let Some(entry) = jobs.by_expression.get(&expression) {
            lock_sinks(&entry.sinks).insert(key, sink);
            return;
        }
        let sinks: Sinks = Arc::new(std::sync::Mutex::new(BTreeMap::from([(key, sink)])));
        let handle = spawn_job(
            schedule,
            expression.clone(),
            sinks.clone(),
            self.lock.clone(),
            self.invoker.clone(),
        );
        jobs.by_expression
            .insert(expression, JobEntry { sinks, handle });
    }

    async fn remove(&self, key: SinkKey) -> bool {
        let mut jobs = self.jobs.lock().await;
        self.take(&mut jobs, &key).await
    }

    /// Takes `key` out of its job, and stops the job when it was the last.
    async fn take(&self, jobs: &mut Jobs, key: &SinkKey) -> bool {
        let Some(expression) = jobs.expression_of.remove(key) else {
            return false;
        };
        let now_empty = jobs.by_expression.get(&expression).is_some_and(|entry| {
            let mut sinks = lock_sinks(&entry.sinks);
            sinks.remove(key);
            sinks.is_empty()
        });
        if now_empty {
            if let Some(entry) = jobs.by_expression.remove(&expression) {
                entry.handle.abort();
                self.lock.release(&expression).await;
            }
        }
        true
    }

    /// Number of running jobs: one per distinct expression.
    pub async fn job_count(&self) -> usize {
        self.jobs.lock().await.by_expression.len()
    }

    /// The trigger bindings, as registered.
    pub async fn job_specs(&self) -> Vec<JobSpec> {
        let jobs = self.jobs.lock().await;
        jobs.by_expression
            .values()
            .flat_map(|entry| {
                lock_sinks(&entry.sinks)
                    .values()
                    .filter_map(|sink| match sink {
                        Sink::Binding(binding) => Some(binding.spec.clone()),
                        Sink::Observer(_) => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Lock-backend hot swap: stops this scheduler's jobs and moves every
    /// binding and observer onto `next`, one job per expression there.
    pub async fn hand_over(&self, next: &Scheduler) {
        let mut jobs = self.jobs.lock().await;
        let mut moved = Vec::new();
        for (expression, entry) in jobs.by_expression.drain() {
            entry.handle.abort();
            self.lock.release(&expression).await;
            let sinks = std::mem::take(&mut *lock_sinks(&entry.sinks));
            moved.push((expression, sinks));
        }
        jobs.expression_of.clear();
        drop(jobs);

        let mut next_jobs = next.jobs.lock().await;
        for (expression, sinks) in moved {
            let schedule = match parse_expression(&expression) {
                Ok(schedule) => schedule,
                Err(e) => {
                    tracing::error!(expression = %expression, error = %e, "failed to move cron job after lock-backend swap");
                    continue;
                }
            };
            for (key, sink) in sinks {
                next.file(
                    &mut next_jobs,
                    schedule.clone(),
                    expression.clone(),
                    key,
                    sink,
                );
            }
        }
    }

    pub async fn shutdown(&self) {
        let mut jobs = self.jobs.lock().await;
        for (expression, entry) in jobs.by_expression.drain() {
            entry.handle.abort();
            self.lock.release(&expression).await;
        }
        jobs.expression_of.clear();
    }
}

fn lock_sinks(sinks: &Sinks) -> std::sync::MutexGuard<'_, BTreeMap<SinkKey, Sink>> {
    sinks.lock().unwrap_or_else(|e| e.into_inner())
}

fn spawn_job(
    schedule: Schedule,
    expression: String,
    sinks: Sinks,
    lock: Arc<dyn CronLock>,
    invoker: Arc<dyn Invoker>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            let Some(next) = schedule.upcoming(Utc).next() else {
                tracing::info!(expression = %expression, "schedule exhausted; stopping job");
                break;
            };
            let wait = (next - now).to_std().unwrap_or(std::time::Duration::ZERO);
            tokio::time::sleep(wait).await;

            // One lock op per expression and fire, however many sinks.
            if !lock.try_acquire(&expression).await {
                continue;
            }

            let tick = CronTick {
                trigger: "cron".to_string(),
                expression: expression.clone(),
                scheduled_time: next.to_rfc3339(),
                actual_time: Utc::now().to_rfc3339(),
            };
            let targets: Vec<Sink> = lock_sinks(&sinks).values().cloned().collect();
            for sink in targets {
                match sink {
                    // Filters are checked inside emit; a miss, or an observer
                    // removed meanwhile, is Ok(false).
                    Sink::Observer(observer) => {
                        if let Err(e) = observer.emit(&tick) {
                            tracing::warn!(observer = %observer.id(), expression = %expression, error = %e, "cron tick emit failed");
                        }
                    }
                    Sink::Binding(binding) => fire_binding(binding, &tick, invoker.clone()),
                }
            }
            lock.release(&expression).await;
        }
    })
}

/// Fires one binding with the trigger type's payload, on its own task so a
/// slow target never delays the other sinks of its expression.
fn fire_binding(binding: Arc<Binding>, tick: &CronTick, invoker: Arc<dyn Invoker>) {
    if binding.in_flight.swap(true, Ordering::AcqRel) {
        tracing::debug!(trigger_id = %binding.spec.trigger_id, "previous cron fire still running; skipping this one");
        return;
    }
    let payload = serde_json::json!({
        "trigger": "cron",
        "job_id": binding.spec.trigger_id,
        "scheduled_time": tick.scheduled_time,
        "actual_time": tick.actual_time,
    });
    tokio::spawn(async move {
        let _busy = InFlight(binding.clone());
        let spec = &binding.spec;
        if let Some(cond) = &spec.condition_function_id {
            // Conditions are plain function calls, not trigger fires — no
            // sidecar.
            match invoker.call(cond, payload.clone(), None).await {
                Ok(Some(v)) if v.as_bool() == Some(false) => return,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(trigger_id = %spec.trigger_id, error = %e, "condition check failed; skipping fire");
                    return;
                }
            }
        }

        if let Err(e) = invoker
            .call(&spec.function_id, payload, spec.metadata.clone())
            .await
        {
            tracing::error!(trigger_id = %spec.trigger_id, function_id = %spec.function_id, error = %e, "cron fire failed");
        }
    });
}

/// Clears a binding's in-flight flag when its fire ends, panics included.
struct InFlight(Arc<Binding>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.store(false, Ordering::Release);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingInvoker {
        calls: AtomicUsize,
        last_payload: Mutex<Option<serde_json::Value>>,
        last_metadata: Mutex<Option<serde_json::Value>>,
        condition_result: Option<serde_json::Value>,
    }

    #[async_trait]
    impl Invoker for CountingInvoker {
        async fn call(
            &self,
            function_id: &str,
            payload: serde_json::Value,
            metadata: Option<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, String> {
            if function_id.starts_with("cond::") {
                return Ok(self.condition_result.clone());
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_payload.lock().await = Some(payload);
            *self.last_metadata.lock().await = metadata;
            Ok(None)
        }
    }

    fn every_second() -> &'static str {
        "*/1 * * * * *"
    }

    fn scheduler_with(invoker: Arc<CountingInvoker>) -> Scheduler {
        Scheduler::new(Arc::new(crate::locks::LocalLock::new()), invoker)
    }

    #[tokio::test]
    async fn register_rejects_empty_expression() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            last_metadata: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv);
        let err = s
            .register(JobSpec {
                trigger_id: "t1".into(),
                expression: "".into(),
                function_id: "f".into(),
                condition_function_id: None,
                metadata: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Cron expression is required"));
    }

    #[tokio::test]
    async fn register_rejects_duplicate_trigger_id() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            last_metadata: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv);
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "f".into(),
            condition_function_id: None,
            metadata: None,
        })
        .await
        .unwrap();
        let err = s
            .register(JobSpec {
                trigger_id: "t1".into(),
                expression: every_second().into(),
                function_id: "f".into(),
                condition_function_id: None,
                metadata: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already registered"));
        s.shutdown().await;
    }

    #[tokio::test]
    async fn fires_within_two_seconds_with_parity_payload() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            last_metadata: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: None,
            metadata: None,
        })
        .await
        .unwrap();
        for _ in 0..25 {
            if inv.calls.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(inv.calls.load(Ordering::SeqCst) >= 1, "job never fired");
        let payload = inv.last_payload.lock().await.clone().unwrap();
        assert_eq!(payload["trigger"], "cron");
        assert_eq!(payload["job_id"], "t1");
        assert!(payload["scheduled_time"].as_str().unwrap().contains('T'));
        assert!(payload["actual_time"].as_str().unwrap().contains('T'));
        s.shutdown().await;
    }

    /// The rctest-k7m3 run-4 failure: the deadline cron fired but the plain
    /// invocation dropped the registration metadata, so harness::notify_agent
    /// could not resolve the subscription and the waiting session never woke.
    #[tokio::test]
    async fn fire_delivers_registration_metadata_sidecar() {
        let inv = Arc::new(CountingInvoker::default());
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: None,
            metadata: Some(serde_json::json!({"subscription_id": "sub_1", "once": true})),
        })
        .await
        .unwrap();
        for _ in 0..25 {
            if inv.calls.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(inv.calls.load(Ordering::SeqCst) >= 1, "job never fired");
        let metadata = inv.last_metadata.lock().await.clone();
        let m = metadata.expect("registration metadata must be delivered at fire time");
        assert_eq!(m["subscription_id"], "sub_1");
        s.shutdown().await;
    }

    #[tokio::test]
    async fn condition_false_blocks_fire() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            last_metadata: Mutex::new(None),
            condition_result: Some(serde_json::json!(false)),
        });
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: Some("cond::c".into()),
            metadata: None,
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        assert_eq!(
            inv.calls.load(Ordering::SeqCst),
            0,
            "condition=false must block"
        );
        s.shutdown().await;
    }

    #[tokio::test]
    async fn unregister_stops_firing() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            last_metadata: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: None,
            metadata: None,
        })
        .await
        .unwrap();
        s.unregister("t1").await.unwrap();
        let before = inv.calls.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        assert_eq!(
            inv.calls.load(Ordering::SeqCst),
            before,
            "unregistered job must not fire"
        );
    }

    // ── shared jobs: bindings and observers on one expression ─────────────

    /// An observer that records every tick it is handed.
    pub(crate) struct RecordingObserver {
        id: String,
        pub(crate) ticks: std::sync::Mutex<Vec<CronTick>>,
    }

    impl RecordingObserver {
        pub(crate) fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                ticks: std::sync::Mutex::new(Vec::new()),
            }
        }

        pub(crate) fn count(&self) -> usize {
            self.ticks.lock().unwrap().len()
        }

        fn last(&self) -> Option<CronTick> {
            self.ticks.lock().unwrap().last().cloned()
        }
    }

    impl TickObserver for RecordingObserver {
        fn id(&self) -> &str {
            &self.id
        }

        fn emit(&self, tick: &CronTick) -> Result<bool, String> {
            self.ticks.lock().unwrap().push(tick.clone());
            Ok(true)
        }
    }

    /// Records each call per function; `slow::*` targets take 2.5 s.
    #[derive(Default)]
    pub(crate) struct RecordingInvoker {
        calls: std::sync::Mutex<Vec<(String, serde_json::Value, Option<serde_json::Value>)>>,
    }

    impl RecordingInvoker {
        fn count(&self, function_id: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(f, _, _)| f == function_id)
                .count()
        }

        fn last(
            &self,
            function_id: &str,
        ) -> Option<(serde_json::Value, Option<serde_json::Value>)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(f, _, _)| f == function_id)
                .map(|(_, p, m)| (p.clone(), m.clone()))
        }
    }

    #[async_trait]
    impl Invoker for RecordingInvoker {
        async fn call(
            &self,
            function_id: &str,
            payload: serde_json::Value,
            metadata: Option<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, String> {
            self.calls
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload, metadata));
            if function_id.starts_with("slow::") {
                tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            }
            if function_id.starts_with("cond::never") {
                return Ok(Some(serde_json::json!(false)));
            }
            Ok(None)
        }
    }

    /// A local lock that counts acquisitions per key.
    #[derive(Default)]
    struct CountingLock {
        inner: crate::locks::LocalLock,
        acquired: std::sync::Mutex<HashMap<String, usize>>,
    }

    #[async_trait]
    impl CronLock for CountingLock {
        async fn try_acquire(&self, job_id: &str) -> bool {
            *self
                .acquired
                .lock()
                .unwrap()
                .entry(job_id.to_string())
                .or_default() += 1;
            self.inner.try_acquire(job_id).await
        }

        async fn release(&self, job_id: &str) {
            self.inner.release(job_id).await
        }
    }

    pub(crate) fn counting_scheduler() -> (Scheduler, Arc<RecordingInvoker>) {
        let invoker = Arc::new(RecordingInvoker::default());
        let scheduler = Scheduler::new(Arc::new(crate::locks::LocalLock::new()), invoker.clone());
        (scheduler, invoker)
    }

    fn binding(trigger_id: &str, expression: &str, function_id: &str) -> JobSpec {
        JobSpec {
            trigger_id: trigger_id.into(),
            expression: expression.into(),
            function_id: function_id.into(),
            condition_function_id: None,
            metadata: None,
        }
    }

    async fn wait_until(what: &str, cond: impl Fn() -> bool) {
        for _ in 0..60 {
            if cond() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn settle() {
        // Past any fire already in progress, then one more period.
        tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    }

    #[tokio::test]
    async fn a_binding_and_an_observer_share_one_job_and_the_observer_outlives_the_binding() {
        let (s, inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("o1"));
        s.register(binding("t1", every_second(), "app::bound"))
            .await
            .unwrap();
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        assert_eq!(s.job_count().await, 1, "one job per distinct expression");

        wait_until("both sinks fired", || {
            inv.count("app::bound") >= 1 && obs.count() >= 1
        })
        .await;

        s.unregister("t1").await.unwrap();
        assert_eq!(s.job_count().await, 1, "the observer keeps the job");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let bound = inv.count("app::bound");
        let seen = obs.count();
        settle().await;
        assert!(obs.count() > seen, "the observer keeps ticking");
        assert_eq!(inv.count("app::bound"), bound, "the binding is gone");

        s.remove_observer("o1").await;
        assert_eq!(s.job_count().await, 0, "the last sink stops the job");
        let seen = obs.count();
        settle().await;
        assert_eq!(obs.count(), seen, "a stopped job fires nothing");
    }

    #[tokio::test]
    async fn a_binding_outlives_an_observer_on_its_expression() {
        let (s, inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("o1"));
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        s.register(binding("t1", every_second(), "app::bound"))
            .await
            .unwrap();
        assert_eq!(s.job_count().await, 1);
        wait_until("both sinks fired", || {
            inv.count("app::bound") >= 1 && obs.count() >= 1
        })
        .await;

        s.remove_observer("o1").await;
        assert_eq!(s.job_count().await, 1, "the binding keeps the job");
        let seen = obs.count();
        let bound = inv.count("app::bound");
        settle().await;
        assert!(inv.count("app::bound") > bound, "the binding keeps firing");
        assert_eq!(obs.count(), seen, "the removed observer hears nothing");

        s.unregister("t1").await.unwrap();
        assert_eq!(s.job_count().await, 0);
    }

    #[tokio::test]
    async fn a_fire_fans_out_to_bindings_and_observers_with_their_own_payloads() {
        let (s, inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("o1"));
        let mut spec = binding("t1", every_second(), "app::bound");
        spec.metadata = Some(serde_json::json!({"subscription_id": "sub_1"}));
        s.register(spec).await.unwrap();
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        wait_until("both sinks fired", || {
            inv.count("app::bound") >= 1 && obs.count() >= 1
        })
        .await;
        s.shutdown().await;

        let (payload, metadata) = inv.last("app::bound").unwrap();
        assert_eq!(payload["trigger"], "cron");
        assert_eq!(payload["job_id"], "t1", "bindings keep their job_id");
        assert_eq!(metadata.unwrap()["subscription_id"], "sub_1");

        let tick = obs.ticks.lock().unwrap()[0].clone();
        assert_eq!(tick.trigger, "cron");
        assert_eq!(tick.expression, every_second());
        let value = serde_json::to_value(&tick).unwrap();
        assert!(value.get("job_id").is_none(), "observers get no job_id");
        // The same fire, seen from both sides.
        assert!(inv.calls.lock().unwrap().iter().any(|(_, p, _)| {
            p["scheduled_time"] == tick.scheduled_time.as_str()
                && p["actual_time"] == tick.actual_time.as_str()
        }));
    }

    #[tokio::test]
    async fn a_false_condition_skips_its_binding_only() {
        let (s, inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("o1"));
        let mut spec = binding("t1", every_second(), "app::bound");
        spec.condition_function_id = Some("cond::never".into());
        s.register(spec).await.unwrap();
        s.register(binding("t2", every_second(), "app::other"))
            .await
            .unwrap();
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        wait_until("two fires", || {
            obs.count() >= 2 && inv.count("app::other") >= 2
        })
        .await;
        s.shutdown().await;
        assert!(
            inv.count("cond::never") >= 2,
            "the condition is still asked"
        );
        assert_eq!(inv.count("app::bound"), 0, "condition=false must block");
    }

    #[tokio::test]
    async fn observers_only_hear_their_own_expression() {
        let (s, _inv) = counting_scheduler();
        let every = Arc::new(RecordingObserver::new("every"));
        let even = Arc::new(RecordingObserver::new("even"));
        s.add_observer(every_second(), every.clone()).await.unwrap();
        s.add_observer("*/2 * * * * *", even.clone()).await.unwrap();
        assert_eq!(s.job_count().await, 2);
        wait_until("both fired", || every.count() >= 3 && even.count() >= 1).await;
        s.shutdown().await;
        assert!(every
            .ticks
            .lock()
            .unwrap()
            .iter()
            .all(|t| t.expression == every_second()));
        for tick in even.ticks.lock().unwrap().iter() {
            assert_eq!(tick.expression, "*/2 * * * * *");
            let at = chrono::DateTime::parse_from_rfc3339(&tick.scheduled_time).unwrap();
            assert_eq!(chrono::Timelike::second(&at) % 2, 0, "{tick:?}");
        }
    }

    #[tokio::test]
    async fn each_fire_takes_the_lock_once_for_its_expression() {
        let lock = Arc::new(CountingLock::default());
        let inv = Arc::new(RecordingInvoker::default());
        let s = Scheduler::new(lock.clone(), inv.clone());
        let obs = Arc::new(RecordingObserver::new("o1"));
        for id in ["t1", "t2", "t3"] {
            s.register(binding(id, every_second(), "app::bound"))
                .await
                .unwrap();
        }
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        s.add_observer(every_second(), Arc::new(RecordingObserver::new("o2")))
            .await
            .unwrap();
        wait_until("three fires", || obs.count() >= 3).await;
        s.shutdown().await;
        // Binding fires run on their own tasks: let the last ones start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let acquired = lock.acquired.lock().unwrap().clone();
        assert_eq!(acquired.len(), 1, "keyed by expression: {acquired:?}");
        assert_eq!(
            acquired[every_second()],
            obs.count(),
            "one lock op per fire, not per sink"
        );
        assert_eq!(inv.count("app::bound"), 3 * obs.count());
    }

    #[tokio::test]
    async fn a_slow_binding_skips_while_busy_and_delays_no_other_sink() {
        let (s, inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("o1"));
        s.register(binding("slow", every_second(), "slow::target"))
            .await
            .unwrap();
        s.register(binding("fast", every_second(), "app::fast"))
            .await
            .unwrap();
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        wait_until("three fires", || {
            obs.count() >= 3 && inv.count("app::fast") >= 3
        })
        .await;
        s.shutdown().await;
        assert_eq!(
            inv.count("slow::target"),
            1,
            "a binding never overlaps itself: busy for 2.5 s, it skips the fires meanwhile"
        );
    }

    #[tokio::test]
    async fn hand_over_moves_every_sink_onto_one_job_per_expression() {
        let (old, old_inv) = counting_scheduler();
        let o1 = Arc::new(RecordingObserver::new("o1"));
        let o2 = Arc::new(RecordingObserver::new("o2"));
        old.register(binding("t1", every_second(), "app::bound"))
            .await
            .unwrap();
        old.register(binding("t2", every_second(), "app::bound"))
            .await
            .unwrap();
        old.add_observer(every_second(), o1.clone()).await.unwrap();
        old.add_observer("*/2 * * * * *", o2.clone()).await.unwrap();

        let (next, next_inv) = counting_scheduler();
        old.hand_over(&next).await;
        assert_eq!(old.job_count().await, 0);
        assert_eq!(next.job_count().await, 2, "one job per expression");
        assert_eq!(next.job_specs().await.len(), 2);

        let old_calls = old_inv.count("app::bound");
        wait_until("moved sinks fire", || {
            next_inv.count("app::bound") >= 2 && o1.count() >= 1 && o2.count() >= 1
        })
        .await;
        assert_eq!(
            old_inv.count("app::bound"),
            old_calls,
            "the old jobs are gone"
        );
        // Ids are still tracked after the move.
        next.unregister("t1").await.unwrap();
        next.unregister("t2").await.unwrap();
        next.remove_observer("o1").await;
        assert_eq!(next.job_count().await, 1);
        next.shutdown().await;
    }

    #[tokio::test]
    async fn an_observer_filed_again_replaces_itself() {
        let (s, _inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("o1"));
        s.add_observer("0 0 9 * * Mon-Fri", obs.clone())
            .await
            .unwrap();
        s.add_observer("0 0 9 * * Mon-Fri", obs.clone())
            .await
            .unwrap();
        assert_eq!(s.job_count().await, 1);
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        assert_eq!(s.job_count().await, 1, "it left its old expression");
        wait_until("a tick", || obs.count() >= 1).await;
        assert_eq!(obs.last().unwrap().expression, every_second());
        s.remove_observer("o1").await;
        assert_eq!(s.job_count().await, 0);
    }

    #[tokio::test]
    async fn trigger_and_observer_ids_never_collide() {
        let (s, inv) = counting_scheduler();
        let obs = Arc::new(RecordingObserver::new("same-id"));
        s.register(binding("same-id", every_second(), "app::bound"))
            .await
            .unwrap();
        s.add_observer(every_second(), obs.clone()).await.unwrap();
        s.unregister("same-id").await.unwrap();
        assert_eq!(s.job_count().await, 1, "the observer is still filed");
        wait_until("observer tick", || obs.count() >= 1).await;
        s.remove_observer("same-id").await;
        assert_eq!(s.job_count().await, 0);
        assert!(s.unregister("same-id").await.is_err());
        let _ = inv;
    }
}
