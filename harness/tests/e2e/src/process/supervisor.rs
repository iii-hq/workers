use std::path::PathBuf;
use std::time::Duration;

use nix::sys::signal::Signal;
use serde::Serialize;

use super::child::{SupervisedChild, REAP_INTERVAL};
use super::spec::ProcessSpec;

pub const DEFAULT_TEARDOWN_BUDGET: Duration = Duration::from_secs(15);
const SIGTERM_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct EarlyExit {
    pub name: String,
    pub status: String,
    pub stderr_log: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeardownIssue {
    pub process: String,
    pub operation: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TeardownReport {
    pub deadline_exceeded: bool,
    pub escalated_to_sigkill: Vec<String>,
    pub remaining_processes: Vec<String>,
    pub issues: Vec<TeardownIssue>,
}

impl TeardownReport {
    pub fn complete(&self) -> bool {
        !self.deadline_exceeded && self.remaining_processes.is_empty() && self.issues.is_empty()
    }
}

pub struct ProcessSupervisor {
    children: Vec<SupervisedChild>,
    teardown_budget: Duration,
}

impl ProcessSupervisor {
    pub fn new(teardown_budget: Duration) -> Self {
        Self {
            children: Vec::new(),
            teardown_budget,
        }
    }

    pub fn set_teardown_budget(&mut self, teardown_budget: Duration) {
        self.teardown_budget = teardown_budget;
    }

    pub fn spawn(&mut self, spec: ProcessSpec) -> anyhow::Result<u32> {
        let child = spec.spawn()?;
        let pid = child.id();
        self.children.push(child);
        Ok(pid)
    }

    #[cfg(test)]
    pub fn remove(&mut self, name: &str) -> Option<SupervisedChild> {
        let index = self
            .children
            .iter()
            .position(|child| child.name() == name)?;
        Some(self.children.remove(index))
    }

    /// The first direct child that has exited. Descendants are still cleaned
    /// up through their process group during teardown.
    pub fn early_exit(&mut self) -> Option<EarlyExit> {
        for supervised in &mut self.children {
            match supervised.try_wait() {
                Ok(Some(status)) => {
                    return Some(EarlyExit {
                        name: supervised.name().to_string(),
                        status: status.to_string(),
                        stderr_log: supervised.stderr_log().to_path_buf(),
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "harness_integration::process",
                        worker = %supervised.name(),
                        "checking child status failed: {error}"
                    );
                }
            }
        }
        None
    }

    /// Wait for a direct child to exit without periodically probing every
    /// process. Register the SIGCHLD listener before the initial status check
    /// so an exit cannot be lost between observation and suspension.
    #[cfg(unix)]
    pub async fn wait_for_exit(&mut self) -> std::io::Result<EarlyExit> {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())?;
        loop {
            if let Some(exit) = self.early_exit() {
                return Ok(exit);
            }
            if signal.recv().await.is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "SIGCHLD signal stream closed",
                ));
            }
        }
    }

    #[cfg(not(unix))]
    pub async fn wait_for_exit(&mut self) -> std::io::Result<EarlyExit> {
        loop {
            if let Some(exit) = self.early_exit() {
                return Ok(exit);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// SIGTERM all process groups in reverse start order, then SIGKILL any
    /// group that survives the grace period. The grace and final reap are
    /// both bounded by `teardown_budget`.
    pub async fn teardown(&mut self) -> TeardownReport {
        let started = tokio::time::Instant::now();
        let final_deadline = started + self.teardown_budget;
        let term_deadline = started + SIGTERM_GRACE.min(self.teardown_budget);
        let mut report = TeardownReport::default();

        for supervised in self.children.iter().rev() {
            if supervised.group_is_alive() {
                if let Err(error) = supervised.signal_tree(Signal::SIGTERM) {
                    tracing::warn!(
                        target: "harness_integration::process",
                        worker = %supervised.name(),
                        "SIGTERM failed: {error}"
                    );
                    report.issues.push(TeardownIssue {
                        process: supervised.name().to_string(),
                        operation: "sigterm".to_string(),
                        error: format!("{error:#}"),
                    });
                }
            }
        }

        self.wait_for_groups_until(term_deadline).await;

        for supervised in self.children.iter().rev() {
            if supervised.group_is_alive() {
                report
                    .escalated_to_sigkill
                    .push(supervised.name().to_string());
                tracing::warn!(
                    target: "harness_integration::process",
                    worker = %supervised.name(),
                    "escalating process group to SIGKILL"
                );
                if let Err(error) = supervised.signal_tree(Signal::SIGKILL) {
                    tracing::warn!(
                        target: "harness_integration::process",
                        worker = %supervised.name(),
                        "SIGKILL failed: {error}"
                    );
                    report.issues.push(TeardownIssue {
                        process: supervised.name().to_string(),
                        operation: "sigkill".to_string(),
                        error: format!("{error:#}"),
                    });
                }
            }
        }

        self.wait_for_groups_until(final_deadline).await;
        for supervised in &mut self.children {
            if supervised.child_is_alive() || supervised.group_is_alive() {
                report
                    .remaining_processes
                    .push(supervised.name().to_string());
            }
        }
        report.deadline_exceeded =
            !report.remaining_processes.is_empty() && tokio::time::Instant::now() >= final_deadline;
        report.issues.extend(self.finalize_direct_children());
        self.children.clear();
        report
    }

    async fn wait_for_groups_until(&mut self, deadline: tokio::time::Instant) {
        loop {
            let mut any_alive = false;
            for supervised in &mut self.children {
                any_alive |= supervised.child_is_alive() || supervised.group_is_alive();
            }
            if !any_alive {
                return;
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return;
            }
            tokio::time::sleep_until((now + REAP_INTERVAL).min(deadline)).await;
        }
    }

    fn finalize_direct_children(&mut self) -> Vec<TeardownIssue> {
        let mut issues = Vec::new();
        for supervised in &mut self.children {
            if let Err(error) = supervised.finalize_direct_child(true) {
                tracing::warn!(
                    target: "harness_integration::process",
                    worker = %supervised.name(),
                    "reaping child failed: {error}"
                );
                issues.push(TeardownIssue {
                    process: supervised.name().to_string(),
                    operation: "reap".to_string(),
                    error: error.to_string(),
                });
                supervised.defer_reap();
            }
        }
        issues
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new(DEFAULT_TEARDOWN_BUDGET)
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        // A boot error can drop a partially populated supervisor before the
        // async teardown path is available. Kill groups, not just leaders,
        // and give the kernel one small bounded scheduling window before
        // handing any stubborn child to the deferred reaper.
        for supervised in self.children.iter().rev() {
            let _ = supervised.signal_tree(Signal::SIGKILL);
        }
        let deadline =
            std::time::Instant::now() + self.teardown_budget.min(Duration::from_millis(100));
        while std::time::Instant::now() < deadline
            && self
                .children
                .iter_mut()
                .any(SupervisedChild::child_is_alive)
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = self.finalize_direct_children();
    }
}
