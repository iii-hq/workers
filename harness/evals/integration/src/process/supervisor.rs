use std::path::PathBuf;
use std::time::Duration;

use nix::sys::signal::Signal;

use super::child::{SupervisedChild, REAP_INTERVAL};
use super::spec::ProcessSpec;

pub const DEFAULT_TEARDOWN_BUDGET: Duration = Duration::from_secs(15);
const SIGTERM_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct EarlyExit {
    pub name: String,
    pub status: String,
    pub stderr_log: PathBuf,
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

    pub fn teardown_budget(&self) -> Duration {
        self.teardown_budget
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

    /// SIGTERM all process groups in reverse start order, then SIGKILL any
    /// group that survives the grace period. The grace and final reap are
    /// both bounded by `teardown_budget`.
    pub async fn teardown(&mut self) {
        let started = tokio::time::Instant::now();
        let final_deadline = started + self.teardown_budget;
        let term_deadline = started + SIGTERM_GRACE.min(self.teardown_budget);

        for supervised in self.children.iter().rev() {
            if supervised.group_is_alive() {
                if let Err(error) = supervised.signal_tree(Signal::SIGTERM) {
                    tracing::warn!(
                        target: "harness_integration::process",
                        worker = %supervised.name(),
                        "SIGTERM failed: {error}"
                    );
                }
            }
        }

        self.wait_for_groups_until(term_deadline).await;

        for supervised in self.children.iter().rev() {
            if supervised.group_is_alive() {
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
                }
            }
        }

        self.wait_for_groups_until(final_deadline).await;
        self.finalize_direct_children();
        self.children.clear();
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

    fn finalize_direct_children(&mut self) {
        for supervised in &mut self.children {
            if let Err(error) = supervised.finalize_direct_child(true) {
                tracing::warn!(
                    target: "harness_integration::process",
                    worker = %supervised.name(),
                    "reaping child failed: {error}"
                );
                supervised.defer_reap();
            }
        }
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
        self.finalize_direct_children();
    }
}
