//! cloudflared Quick Tunnel child processes.
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use futures_util::StreamExt;
use tokio::{
    io::AsyncRead,
    process::{Child, Command},
    task::JoinHandle,
};
use tokio_util::codec::{FramedRead, LinesCodec};

use super::{Provider, Session, Signal, Sink};
use crate::{
    parser::{self, LogEvent},
    prerequisites::{self, Prerequisites, ProbeCache},
};

pub struct Cloudflared {
    executable: String,
    probe: ProbeCache,
}

impl Cloudflared {
    pub fn new(executable: &str) -> Self {
        Self {
            executable: executable.to_owned(),
            probe: ProbeCache::default(),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Cloudflared {
    fn name(&self) -> &'static str {
        "cloudflared"
    }

    fn start(
        &self,
        _tunnel_id: &str,
        origin: &str,
        state_dir: &Path,
        sink: Sink,
    ) -> Result<Box<dyn Session>, String> {
        // Only a binary that does not resolve is "not found"; any other
        // spawn error (state dir gone, permissions) gets the generic one.
        if prerequisites::resolve(&self.executable).is_none() {
            return Err(prerequisites::not_found_error(&self.executable));
        }
        match spawn(&self.executable, origin, state_dir, sink) {
            Ok(process) => Ok(Box::new(process)),
            Err(e) => Err(format!(
                "cloudflared spawn failed ({:?}); verify prerequisite executable",
                e.kind()
            )),
        }
    }

    async fn prerequisites(&self) -> Option<Prerequisites> {
        // Probed outside the owning task: a slow `--version` never stalls leases.
        Some(Prerequisites {
            cloudflared: self.probe.check(&self.executable).await,
        })
    }
}

struct Process {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    _home: tempfile::TempDir,
}
impl Drop for Process {
    fn drop(&mut self) {
        // Child::kill_on_drop is the final safety net if the owning task is cancelled.
        for reader in &self.readers {
            reader.abort();
        }
    }
}

#[async_trait::async_trait]
impl Session for Process {
    fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("cloudflared exited ({status})")),
            Err(_) => Some("cloudflared wait failed".into()),
            Ok(None) => None,
        }
    }

    async fn stop(mut self: Box<Self>) {
        if let Err(e) = self.child.kill().await {
            tracing::warn!(error = %e, "child kill failed; waiting for reap");
        }
        if let Err(e) = self.child.wait().await {
            tracing::warn!(error = %e, "child reap failed");
        }
        for reader in &self.readers {
            reader.abort();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.await;
        }
    }
}

fn spawn(executable: &str, origin: &str, state_dir: &Path, sink: Sink) -> std::io::Result<Process> {
    let home = tempfile::Builder::new()
        .prefix("cloudflared-")
        .tempdir_in(state_dir)?;
    let home_path: PathBuf = home.path().canonicalize()?;
    let empty_config = home_path.join("config.yaml");
    std::fs::write(&empty_config, "{}\n")?;
    // No inherited TUNNEL_*, proxy variables, credentials, or home config.
    // Resolve first: the child PATH is narrow and misses e.g. /opt/homebrew/bin.
    let program = prerequisites::resolve(executable)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let mut child = Command::new(program)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &home_path)
        .current_dir(&home_path)
        .args(["tunnel", "--config"])
        .arg(empty_config)
        .args([
            "--no-autoupdate",
            "--output",
            "json",
            "--loglevel",
            "info",
            "--metrics",
            "127.0.0.1:0",
            "--retries",
            "0",
            "--grace-period",
            "0s",
            "--url",
        ])
        .arg(origin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(read_output(stdout, sink.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(read_output(stderr, sink));
    }
    Ok(Process {
        child,
        readers,
        _home: home,
    })
}

fn read_output<R: AsyncRead + Unpin + Send + 'static>(reader: R, sink: Sink) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(16_384));
        while let Some(line) = lines.next().await {
            let (signal, broken) = match line {
                Ok(line) => (parser::parse(&line).map(signal), false),
                Err(_) => (
                    Some(Signal::Fatal(
                        "cloudflared log stream invalid or oversized".into(),
                    )),
                    true,
                ),
            };
            if let Some(signal) = signal {
                if !sink.send(signal).await {
                    break;
                }
            }
            if broken {
                break;
            }
        }
    })
}

fn signal(event: LogEvent) -> Signal {
    match event {
        LogEvent::Url(url) => Signal::Url(url),
        LogEvent::Connected => Signal::Connected,
        LogEvent::Disconnected => Signal::Disconnected,
    }
}
