//! Process spawning and teardown for isolated integration stacks.
//!
//! Every spawned process becomes the leader of a new process group. Signals
//! are sent to the group rather than only to the direct child, so helper
//! processes cannot survive a scenario teardown or a partially failed boot.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use anyhow::Context;
use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;

pub const DEFAULT_TEARDOWN_BUDGET: Duration = Duration::from_secs(15);
const SIGTERM_GRACE: Duration = Duration::from_secs(5);
const REAP_INTERVAL: Duration = Duration::from_millis(25);
const IMMEDIATE_KILL_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub name: String,
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
}

impl ProcessSpec {
    pub fn new(
        name: impl Into<String>,
        executable: impl Into<PathBuf>,
        cwd: impl Into<PathBuf>,
        stdout_log: impl Into<PathBuf>,
        stderr_log: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            executable: executable.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            stdout_log: stdout_log.into(),
            stderr_log: stderr_log.into(),
            env: BTreeMap::new(),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs<I, K, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.extend(
            values
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    fn spawn(self) -> anyhow::Result<SupervisedChild> {
        create_parent(&self.stdout_log)?;
        create_parent(&self.stderr_log)?;
        let stdout = std::fs::File::create(&self.stdout_log)
            .with_context(|| format!("creating {}", self.stdout_log.display()))?;
        let stderr = std::fs::File::create(&self.stderr_log)
            .with_context(|| format!("creating {}", self.stderr_log.display()))?;

        let mut command = Command::new(&self.executable);
        command
            .args(&self.args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .env_clear()
            .envs(&self.env)
            // Zero asks the kernel to use the child's PID as its process
            // group ID. This runs between fork and exec without a closure.
            .process_group(0);

        let child = command.spawn().with_context(|| {
            format!("spawning {} from {}", self.name, self.executable.display())
        })?;
        let process_group = Pid::from_raw(child.id() as i32);
        tracing::info!(worker = %self.name, pid = child.id(), pgid = process_group.as_raw(), "spawned");

        Ok(SupervisedChild {
            name: self.name,
            child,
            process_group,
            stderr_log: self.stderr_log,
            cleaned_up: false,
        })
    }
}

fn create_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    Ok(())
}

pub struct SupervisedChild {
    name: String,
    child: Child,
    process_group: Pid,
    stderr_log: PathBuf,
    cleaned_up: bool,
}

impl SupervisedChild {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn stderr_log(&self) -> &Path {
        &self.stderr_log
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub async fn kill_now(&mut self) -> anyhow::Result<()> {
        self.signal_tree(Signal::SIGKILL)?;
        let deadline = tokio::time::Instant::now() + IMMEDIATE_KILL_BUDGET;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.cleaned_up = true;
                    return Ok(());
                }
                Ok(None) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(REAP_INTERVAL).await;
                }
                Ok(None) => {
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

    fn signal_group(&self, signal: Signal) -> anyhow::Result<()> {
        match killpg(self.process_group, signal) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!("sending {signal:?} to process group {}", self.process_group)
            }),
        }
    }

    /// Signal both the process group and its original leader. The direct
    /// signal covers a child that escaped its group after startup.
    fn signal_tree(&self, signal: Signal) -> anyhow::Result<()> {
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

    fn group_is_alive(&self) -> bool {
        match killpg(self.process_group, None) {
            Ok(()) | Err(Errno::EPERM) => true,
            Err(Errno::ESRCH) => false,
            Err(error) => {
                tracing::warn!(worker = %self.name, "checking process group failed: {error}");
                true
            }
        }
    }

    fn child_is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => {
                self.cleaned_up = true;
                false
            }
            Ok(None) => true,
            Err(error) => {
                tracing::warn!(worker = %self.name, "checking child failed: {error}");
                true
            }
        }
    }

    /// Hand a stubborn direct child to a background `waitpid`, allowing the
    /// async teardown deadline to remain a hard upper bound without leaking
    /// a zombie.
    fn defer_reap(&mut self) {
        if self.cleaned_up {
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
            tracing::warn!(worker = %self.name, "could not start deferred reaper: {error}");
        }
        self.cleaned_up = true;
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        if self.cleaned_up {
            return;
        }
        let _ = self.signal_tree(Signal::SIGKILL);
        let _ = self.child.kill();
        match self.child.try_wait() {
            Ok(Some(_)) => self.cleaned_up = true,
            Ok(None) | Err(_) => self.defer_reap(),
        }
    }
}

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
        let index = self.children.iter().position(|child| child.name == name)?;
        Some(self.children.remove(index))
    }

    /// The first direct child that has exited. Descendants are still cleaned
    /// up through their process group during teardown.
    pub fn early_exit(&mut self) -> Option<EarlyExit> {
        for supervised in &mut self.children {
            match supervised.try_wait() {
                Ok(Some(status)) => {
                    return Some(EarlyExit {
                        name: supervised.name.clone(),
                        status: status.to_string(),
                        stderr_log: supervised.stderr_log.clone(),
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(worker = %supervised.name, "checking child status failed: {error}");
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
                    tracing::warn!(worker = %supervised.name, "SIGTERM failed: {error}");
                }
            }
        }

        self.wait_for_groups_until(term_deadline).await;

        for supervised in self.children.iter().rev() {
            if supervised.group_is_alive() {
                tracing::warn!(worker = %supervised.name, "escalating process group to SIGKILL");
                if let Err(error) = supervised.signal_tree(Signal::SIGKILL) {
                    tracing::warn!(worker = %supervised.name, "SIGKILL failed: {error}");
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
            match supervised.child.try_wait() {
                Ok(Some(_)) => supervised.cleaned_up = true,
                Ok(None) => {
                    let _ = supervised.signal_tree(Signal::SIGKILL);
                    let _ = supervised.child.kill();
                    supervised.defer_reap();
                }
                Err(error) => {
                    tracing::warn!(worker = %supervised.name, "reaping child failed: {error}");
                    supervised.defer_reap();
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn builder_preserves_args_and_environment() {
        let spec = ProcessSpec::new("worker", "/bin/echo", "/", "/tmp/out", "/tmp/err")
            .args(["one", "two"])
            .env("LANG", "C");
        assert_eq!(spec.args, [OsStr::new("one"), OsStr::new("two")]);
        assert_eq!(spec.env.get(OsStr::new("LANG")), Some(&OsString::from("C")));
    }

    #[test]
    fn drop_cleans_up_a_partially_started_supervisor() {
        let dir = tempfile::tempdir().unwrap();
        let mut supervisor = ProcessSupervisor::new(Duration::from_millis(100));
        let running = ProcessSpec::new(
            "running",
            "/bin/sh",
            dir.path(),
            dir.path().join("running.out"),
            dir.path().join("running.err"),
        )
        .args(["-c", "exec sleep 60"]);
        let pid = supervisor.spawn(running).unwrap();

        let invalid = ProcessSpec::new(
            "missing",
            dir.path().join("does-not-exist"),
            dir.path(),
            dir.path().join("missing.out"),
            dir.path().join("missing.err"),
        );
        assert!(supervisor.spawn(invalid).is_err());

        drop(supervisor);
        assert!(
            !process_is_running(pid),
            "first child survived a later spawn failure"
        );
    }

    fn process_is_running(pid: u32) -> bool {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(_) => return false,
        };
        // A zombie has already stopped executing and only awaits its parent
        // reading the exit status.
        stat.rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_some_and(|state| state != 'Z')
    }
}
