//! Scheduling loop: one tokio task per distinct cron expression, retained by
//! the `cron::on-tick` observers of that expression. Port of the builtin's
//! structs.rs: UTC only; no catch-up (always the NEXT upcoming fire from now
//! -- missed runs while down are skipped, never replayed).
//!
//! Each fire hands one [`CronTick`] to the scheduler's [`Ticker`]: the worker
//! calls `cron::on-tick` with it once, and the function's match hook gives it
//! to the observers of that expression. The job stops when its last observer
//! releases it.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::on_tick::CronTick;

/// Where each fire goes: the worker calls `cron::on-tick` with it
/// ([`crate::on_tick::OnTickCaller`]), tests record it. Called on the job's
/// task, so it returns at once.
pub trait Ticker: Send + Sync + 'static {
    fn tick(&self, tick: CronTick);
}

/// One running job and the observers that retain it.
struct Job {
    observers: HashSet<String>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct Jobs {
    by_expression: HashMap<String, Job>,
    /// The expression each observer retains, by observer id.
    expression_of: HashMap<String, String>,
}

pub struct Scheduler {
    ticker: Arc<dyn Ticker>,
    jobs: Mutex<Jobs>,
}

/// Parses an observer's expression; the message is the rejection's.
pub fn parse_expression(expression: &str) -> anyhow::Result<Schedule> {
    if expression.trim().is_empty() {
        anyhow::bail!("Cron expression is required");
    }
    Schedule::from_str(expression)
        .map_err(|e| anyhow::anyhow!("invalid cron expression '{expression}': {e}"))
}

impl Scheduler {
    pub fn new(ticker: Arc<dyn Ticker>) -> Self {
        Self {
            ticker,
            jobs: Mutex::new(Jobs::default()),
        }
    }

    /// Retains the job of `expression` for `observer_id`, starting it when it
    /// is the first. An observer retained again under the same expression
    /// keeps one place; under another expression it leaves its old one.
    pub async fn retain(&self, observer_id: &str, expression: &str) -> anyhow::Result<()> {
        let schedule = parse_expression(expression)?;
        let mut jobs = self.jobs.lock().await;
        if let Some(held) = jobs.expression_of.get(observer_id) {
            if held == expression {
                return Ok(());
            }
            Self::take(&mut jobs, observer_id);
        }
        jobs.expression_of
            .insert(observer_id.to_string(), expression.to_string());
        if let Some(job) = jobs.by_expression.get_mut(expression) {
            job.observers.insert(observer_id.to_string());
            return Ok(());
        }
        let handle = spawn_job(schedule, expression.to_string(), self.ticker.clone());
        jobs.by_expression.insert(
            expression.to_string(),
            Job {
                observers: HashSet::from([observer_id.to_string()]),
                handle,
            },
        );
        Ok(())
    }

    /// Releases the job `observer_id` retains; the job stops with its last
    /// observer. `false` when the observer retained nothing.
    pub async fn release(&self, observer_id: &str) -> bool {
        let mut jobs = self.jobs.lock().await;
        if !jobs.expression_of.contains_key(observer_id) {
            return false;
        }
        Self::take(&mut jobs, observer_id);
        true
    }

    /// Takes `observer_id` out of its job, and stops the job when it was the
    /// last.
    fn take(jobs: &mut Jobs, observer_id: &str) {
        let Some(expression) = jobs.expression_of.remove(observer_id) else {
            return;
        };
        let now_empty = jobs.by_expression.get_mut(&expression).is_some_and(|job| {
            job.observers.remove(observer_id);
            job.observers.is_empty()
        });
        if now_empty {
            if let Some(job) = jobs.by_expression.remove(&expression) {
                job.handle.abort();
            }
        }
    }

    /// Number of running jobs: one per distinct expression.
    pub async fn job_count(&self) -> usize {
        self.jobs.lock().await.by_expression.len()
    }

    /// Stops every job and forgets every observer.
    pub async fn shutdown(&self) {
        let mut jobs = self.jobs.lock().await;
        for (_, job) in jobs.by_expression.drain() {
            job.handle.abort();
        }
        jobs.expression_of.clear();
    }
}

fn spawn_job(schedule: Schedule, expression: String, ticker: Arc<dyn Ticker>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            let Some(next) = schedule.upcoming(Utc).next() else {
                tracing::info!(expression = %expression, "schedule exhausted; stopping job");
                break;
            };
            let wait = (next - now).to_std().unwrap_or(std::time::Duration::ZERO);
            tokio::time::sleep(wait).await;

            ticker.tick(CronTick {
                expression: expression.clone(),
                scheduled_time: next.to_rfc3339(),
                actual_time: Utc::now().to_rfc3339(),
            });
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Records every tick it is handed.
    #[derive(Default)]
    pub(crate) struct RecordingTicker {
        pub(crate) ticks: std::sync::Mutex<Vec<CronTick>>,
    }

    impl RecordingTicker {
        pub(crate) fn count(&self, expression: &str) -> usize {
            self.ticks
                .lock()
                .unwrap()
                .iter()
                .filter(|tick| tick.expression == expression)
                .count()
        }

        fn of(&self, expression: &str) -> Vec<CronTick> {
            self.ticks
                .lock()
                .unwrap()
                .iter()
                .filter(|tick| tick.expression == expression)
                .cloned()
                .collect()
        }
    }

    impl Ticker for RecordingTicker {
        fn tick(&self, tick: CronTick) {
            self.ticks.lock().unwrap().push(tick);
        }
    }

    pub(crate) fn recording_scheduler() -> (Scheduler, Arc<RecordingTicker>) {
        let ticker = Arc::new(RecordingTicker::default());
        (Scheduler::new(ticker.clone()), ticker)
    }

    const EVERY_SECOND: &str = "*/1 * * * * *";
    const EVEN_SECONDS: &str = "*/2 * * * * *";

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
    async fn retain_rejects_an_empty_or_invalid_expression() {
        let (s, _ticker) = recording_scheduler();
        for (expression, message) in [
            ("", "Cron expression is required"),
            ("   ", "Cron expression is required"),
            ("not a cron", "invalid cron expression 'not a cron'"),
            ("* * * * *", "invalid cron expression '* * * * *'"),
        ] {
            let err = s.retain("o1", expression).await.unwrap_err();
            assert!(err.to_string().contains(message), "{expression:?}: {err}");
        }
        assert_eq!(s.job_count().await, 0);
        assert!(!s.release("o1").await, "a refused observer retains nothing");
    }

    #[tokio::test]
    async fn a_job_ticks_with_its_expression_and_both_times() {
        let (s, ticker) = recording_scheduler();
        s.retain("o1", EVERY_SECOND).await.unwrap();
        wait_until("a tick", || ticker.count(EVERY_SECOND) >= 1).await;
        s.shutdown().await;
        let tick = ticker.of(EVERY_SECOND)[0].clone();
        assert_eq!(tick.expression, EVERY_SECOND);
        let scheduled = chrono::DateTime::parse_from_rfc3339(&tick.scheduled_time).unwrap();
        let actual = chrono::DateTime::parse_from_rfc3339(&tick.actual_time).unwrap();
        assert!(actual >= scheduled, "{tick:?}");
    }

    /// Filters compare strings by code point, so a time bound orders ticks
    /// only when it is written in the form the worker emits.
    #[tokio::test]
    async fn tick_times_are_rfc3339_with_a_plus_zero_offset() {
        let (s, ticker) = recording_scheduler();
        s.retain("o1", EVERY_SECOND).await.unwrap();
        wait_until("a tick", || ticker.count(EVERY_SECOND) >= 1).await;
        s.shutdown().await;
        let tick = ticker.of(EVERY_SECOND)[0].clone();
        for time in [&tick.scheduled_time, &tick.actual_time] {
            assert!(time.ends_with("+00:00"), "{time}");
        }
        // Whole seconds: `YYYY-MM-DDTHH:MM:SS+00:00`.
        assert_eq!(
            tick.scheduled_time.len(),
            "2026-11-01T00:00:00+00:00".len(),
            "{}",
            tick.scheduled_time
        );
        // A `$gte` bound at the fire's own time, in the emitted form, keeps
        // it; the same instant written with `Z` sorts after it.
        let at = tick.scheduled_time.as_str();
        assert!(at >= at.to_string().as_str());
        assert!(at < at.replace("+00:00", "Z").as_str());
    }

    /// Each filter example on `input.scheduled_time` or `input.actual_time`
    /// in the README and SKILL.md bounds it with a time in the emitted form.
    #[test]
    fn the_docs_write_time_bounds_in_the_emitted_form() {
        for (doc, text) in [
            ("README.md", include_str!("../README.md")),
            ("skills/SKILL.md", include_str!("../skills/SKILL.md")),
        ] {
            let mut bounds = 0;
            let examples = text.lines().filter(|line| {
                line.contains("'input.scheduled_time'") || line.contains("'input.actual_time'")
            });
            for line in examples {
                let quoted = line.split('\'').skip(1).step_by(2);
                for time in quoted.filter(|q| q.starts_with(|c: char| c.is_ascii_digit())) {
                    let parsed = chrono::DateTime::parse_from_rfc3339(time)
                        .unwrap_or_else(|e| panic!("{doc}: {time}: {e}"));
                    assert_eq!(
                        parsed.with_timezone(&Utc).to_rfc3339(),
                        time,
                        "{doc}: {time} is not in the form the worker emits"
                    );
                    bounds += 1;
                }
            }
            assert!(bounds > 0, "{doc} shows a time filter");
        }
    }

    #[tokio::test]
    async fn observers_of_one_expression_share_one_job_that_ticks_once_per_fire() {
        let (s, ticker) = recording_scheduler();
        for id in ["o1", "o2", "o3"] {
            s.retain(id, EVERY_SECOND).await.unwrap();
        }
        assert_eq!(s.job_count().await, 1, "one job per distinct expression");
        wait_until("three fires", || ticker.count(EVERY_SECOND) >= 3).await;
        s.shutdown().await;
        let ticks = ticker.of(EVERY_SECOND);
        let fires: std::collections::BTreeSet<_> =
            ticks.iter().map(|t| t.scheduled_time.clone()).collect();
        assert_eq!(fires.len(), ticks.len(), "one tick per fire: {ticks:?}");
    }

    #[tokio::test]
    async fn each_expression_ticks_on_its_own_schedule() {
        let (s, ticker) = recording_scheduler();
        s.retain("every", EVERY_SECOND).await.unwrap();
        s.retain("even", EVEN_SECONDS).await.unwrap();
        assert_eq!(s.job_count().await, 2);
        wait_until("both ticked", || {
            ticker.count(EVERY_SECOND) >= 3 && ticker.count(EVEN_SECONDS) >= 1
        })
        .await;
        s.shutdown().await;
        for tick in ticker.of(EVEN_SECONDS) {
            let at = chrono::DateTime::parse_from_rfc3339(&tick.scheduled_time).unwrap();
            assert_eq!(chrono::Timelike::second(&at) % 2, 0, "{tick:?}");
        }
    }

    #[tokio::test]
    async fn the_job_stops_with_its_last_observer() {
        let (s, ticker) = recording_scheduler();
        s.retain("o1", EVERY_SECOND).await.unwrap();
        s.retain("o2", EVERY_SECOND).await.unwrap();
        wait_until("a tick", || ticker.count(EVERY_SECOND) >= 1).await;

        assert!(s.release("o1").await);
        assert_eq!(s.job_count().await, 1, "o2 keeps the job");
        let seen = ticker.count(EVERY_SECOND);
        settle().await;
        assert!(ticker.count(EVERY_SECOND) > seen, "the job keeps ticking");

        assert!(s.release("o2").await);
        assert_eq!(s.job_count().await, 0, "the last observer stops the job");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let seen = ticker.count(EVERY_SECOND);
        settle().await;
        assert_eq!(
            ticker.count(EVERY_SECOND),
            seen,
            "a stopped job ticks no more"
        );
    }

    #[tokio::test]
    async fn releasing_an_unknown_observer_changes_nothing() {
        let (s, _ticker) = recording_scheduler();
        s.retain("o1", EVERY_SECOND).await.unwrap();
        assert!(!s.release("ghost").await);
        assert_eq!(s.job_count().await, 1);
        s.shutdown().await;
    }

    #[tokio::test]
    async fn an_observer_retained_again_keeps_one_place() {
        let (s, ticker) = recording_scheduler();
        s.retain("o1", "0 0 9 * * Mon-Fri").await.unwrap();
        s.retain("o1", "0 0 9 * * Mon-Fri").await.unwrap();
        assert_eq!(s.job_count().await, 1);

        s.retain("o1", EVERY_SECOND).await.unwrap();
        assert_eq!(s.job_count().await, 1, "it left its old expression");
        wait_until("a tick", || ticker.count(EVERY_SECOND) >= 1).await;

        assert!(s.release("o1").await);
        assert_eq!(s.job_count().await, 0);
        assert!(!s.release("o1").await, "released once");
    }

    #[tokio::test]
    async fn shutdown_stops_every_job() {
        let (s, ticker) = recording_scheduler();
        s.retain("o1", EVERY_SECOND).await.unwrap();
        s.retain("o2", EVEN_SECONDS).await.unwrap();
        s.shutdown().await;
        assert_eq!(s.job_count().await, 0);
        assert!(!s.release("o1").await, "observers are forgotten");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let seen = ticker.ticks.lock().unwrap().len();
        settle().await;
        assert_eq!(ticker.ticks.lock().unwrap().len(), seen);
    }
}
