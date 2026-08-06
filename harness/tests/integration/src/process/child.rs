use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::Duration;

use anyhow::Context;
use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;

pub(super) const REAP_INTERVAL: Duration = Duration::from_millis(25);
const IMMEDIATE_KILL_BUDGET: Duration = Duration::from_secs(2);

/// Tracks who owns the final wait for the direct child.
///
/// `Running` means this value must still finalize the child on drop. `Reaped`
/// means a synchronous wait completed, while `Deferred` means a background
/// waitpid owns the final reap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalizationState {
    Running,
    Reaped,
    Deferred,
}

pub struct SupervisedChild {
    name: String,
    child: Child,
    process_group: Pid,
    stderr_log: PathBuf,
    finalization: FinalizationState,
}

impl SupervisedChild {
    pub(super) fn new(name: String, child: Child, process_group: Pid, stderr_log: PathBuf) -> Self {
        Self {
            name,
            child,
            process_group,
            stderr_log,
            finalization: FinalizationState::Running,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn stderr_log(&self) -> &Path {
        &self.stderr_log
    }

    /// Observe the direct child without transferring process-tree cleanup
    /// ownership. A removed child must still clean descendants up on drop.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub async fn kill_now(&mut self) -> anyhow::Result<()> {
        self.signal_tree(Signal::SIGKILL)?;
        let deadline = tokio::time::Instant::now() + IMMEDIATE_KILL_BUDGET;
        loop {
            match self.poll_reap() {
                Ok(true) => return Ok(()),
                Ok(false) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(REAP_INTERVAL).await;
                }
                Ok(false) => {
                    let _ = self.child.kill();
                    self.defer_reap();
                    anyhow::bail!(
                        "{} did not reap within {}ms after SIGKILL",
                        self.name,
                        IMMEDIATE_KILL_BUDGET.as_millis()
                    );
                }
                Err(error) => {
                    self.defer_reap();
                    return Err(error).with_context(|| format!("reaping {}", self.name));
                }
            }
        }
    }

    pub(super) fn signal_tree(&self, signal: Signal) -> anyhow::Result<()> {
        let group = self.signal_group(signal);
        let direct = match kill(Pid::from_raw(self.child.id() as i32), signal) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("sending {signal:?} directly to {}", self.name))
            }
        };
        match (group, direct) {
            (Ok(()), _) | (_, Ok(())) => Ok(()),
            (Err(group_error), Err(direct_error)) => Err(anyhow::anyhow!(
                "{group_error:#}; direct signal also failed: {direct_error:#}"
            )),
        }
    }

    pub(super) fn group_is_alive(&self) -> bool {
        match killpg(self.process_group, None) {
            Ok(()) | Err(Errno::EPERM) => true,
            Err(Errno::ESRCH) => false,
            Err(error) => {
                tracing::warn!(
                    target: "harness_integration::process",
                    worker = %self.name,
                    "checking process group failed: {error}"
                );
                true
            }
        }
    }

    pub(super) fn child_is_alive(&mut self) -> bool {
        match self.poll_reap() {
            Ok(true) => false,
            Ok(false) => true,
            Err(error) => {
                tracing::warn!(
                    target: "harness_integration::process",
                    worker = %self.name,
                    "checking child failed: {error}"
                );
                true
            }
        }
    }

    /// Complete direct-child cleanup after process-tree signalling. When
    /// `signal_if_running` is true, a still-running leader receives the same
    /// final SIGKILL fallback used by supervisor teardown.
    pub(super) fn finalize_direct_child(&mut self, signal_if_running: bool) -> std::io::Result<()> {
        match self.poll_reap() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if signal_if_running {
                    let _ = self.signal_tree(Signal::SIGKILL);
                    let _ = self.child.kill();
                }
                self.defer_reap();
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn signal_group(&self, signal: Signal) -> anyhow::Result<()> {
        match killpg(self.process_group, signal) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!("sending {signal:?} to process group {}", self.process_group)
            }),
        }
    }

    fn poll_reap(&mut self) -> std::io::Result<bool> {
        match self.child.try_wait()? {
            Some(_) => {
                self.finalization = FinalizationState::Reaped;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Hand a stubborn direct child to a background `waitpid`, allowing the
    /// async teardown deadline to remain a hard upper bound without leaking
    /// a zombie.
    pub(super) fn defer_reap(&mut self) {
        if self.finalization != FinalizationState::Running {
            return;
        }
        let pid = Pid::from_raw(self.child.id() as i32);
        let name = self.name.clone();
        if let Err(error) = std::thread::Builder::new()
            .name(format!("reap-{name}"))
            .spawn(move || {
                let _ = waitpid(pid, None);
            })
        {
            tracing::warn!(
                target: "harness_integration::process",
                worker = %self.name,
                "could not start deferred reaper: {error}"
            );
        }
        self.finalization = FinalizationState::Deferred;
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        if self.finalization != FinalizationState::Running {
            return;
        }
        let _ = self.signal_tree(Signal::SIGKILL);
        let _ = self.child.kill();
        if self.finalize_direct_child(false).is_err() {
            self.defer_reap();
        }
    }
}
