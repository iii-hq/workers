use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;
use nix::unistd::Pid;

use super::child::SupervisedChild;

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

    pub(super) fn spawn(self) -> anyhow::Result<SupervisedChild> {
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

        // Coverage runs need instrumented children to keep writing their
        // profiles despite env_clear; inert otherwise.
        if let Ok(profile_file) = std::env::var("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile_file);
        }

        let child = command.spawn().with_context(|| {
            format!("spawning {} from {}", self.name, self.executable.display())
        })?;
        let process_group = Pid::from_raw(child.id() as i32);
        tracing::info!(
            target: "harness_integration::process",
            worker = %self.name,
            pid = child.id(),
            pgid = process_group.as_raw(),
            "spawned"
        );

        Ok(SupervisedChild::new(
            self.name,
            child,
            process_group,
            self.stderr_log,
        ))
    }
}

fn create_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    Ok(())
}
