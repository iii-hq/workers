//! Diagnostics-only console screen recording. When `--record-console` is
//! set (and a console worker is in the stack), the runner spawns
//! `tools/console-recorder/record.mjs` in headless system Chrome for the
//! Send→Collect window and stores `console-recording.webm` next to the
//! scenario evidence. Per the spec's traces stance the recording is never
//! an oracle: a recorder failure is logged and costs the artifact, not the
//! scenario.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::Context;

pub struct ConsoleRecording {
    child: Child,
    pub output: PathBuf,
    log: PathBuf,
    started: std::time::Instant,
}

/// Fast scenarios can reach Collect before headless Chrome has even
/// finished launching; stopping then yields no video at all. Hold the
/// recorder for at least this long so the capture always includes the
/// final UI state.
const MIN_CAPTURE: std::time::Duration = std::time::Duration::from_secs(8);

impl ConsoleRecording {
    /// Wait until the console worker answers HTTP on `/` (readiness for the
    /// recording profile; plain TCP + minimal GET, no client dependency).
    pub async fn wait_http_ready(port: u16, deadline_ms: u64) -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(deadline_ms);
        loop {
            let attempt = async {
                let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .ok()?;
                stream
                    .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .await
                    .ok()?;
                let mut head = [0u8; 15];
                stream.read_exact(&mut head).await.ok()?;
                std::str::from_utf8(&head)
                    .ok()
                    .filter(|h| h.contains(" 200"))
                    .map(|_| ())
            };
            if attempt.await.is_some() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("console HTTP on 127.0.0.1:{port} not ready in {deadline_ms}ms");
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    /// Spawn the recorder. `script_dir` is `tools/console-recorder` (must
    /// have its node_modules installed beforehand — the runner never
    /// downloads during a test).
    pub fn start(
        script_dir: &Path,
        console_url: &str,
        output: &Path,
        chrome: &Path,
        session_id: &str,
    ) -> anyhow::Result<ConsoleRecording> {
        let script = script_dir.join("record.mjs");
        if !script.is_file() {
            anyhow::bail!("recorder script missing at {}", script.display());
        }
        if !script_dir.join("node_modules").is_dir() {
            anyhow::bail!(
                "recorder dependencies not installed; run `pnpm install` in {}",
                script_dir.display()
            );
        }
        let log = output.with_extension("recorder.log");
        let log_file = std::fs::File::create(&log)?;
        let child = Command::new("node")
            .arg(&script)
            .args(["--url", console_url])
            .args(["--out", &output.to_string_lossy()])
            .args(["--chrome", &chrome.to_string_lossy()])
            .args(["--session", session_id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file.try_clone()?))
            .spawn()
            .context("spawning console recorder (is `node` on PATH?)")?;
        tracing::info!(
            pid = child.id(),
            url = console_url,
            "console recorder started"
        );
        Ok(ConsoleRecording {
            child,
            output: output.to_path_buf(),
            log,
            started: std::time::Instant::now(),
        })
    }

    /// Hold until the recorder's page has loaded (its log carries a
    /// `recorder: page loaded` marker), so Send never outruns Chrome's
    /// startup on a fast scenario. Best-effort: on timeout the scenario
    /// proceeds and the video just starts late (or not at all).
    pub async fn wait_page_loaded(&self, timeout_ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if std::fs::read_to_string(&self.log)
                .is_ok_and(|log| log.contains("recorder: page loaded"))
            {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("console recorder page never loaded; recording may be empty");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// SIGTERM the recorder (it closes the browser context, which flushes
    /// the video) and wait for the file to land.
    pub async fn stop(mut self) -> Option<PathBuf> {
        self.stop_inner().await
    }

    async fn stop_inner(&mut self) -> Option<PathBuf> {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        if let Some(remaining) = MIN_CAPTURE.checked_sub(self.started.elapsed()) {
            tokio::time::sleep(remaining).await;
        }
        let pid = Pid::from_raw(self.child.id() as i32);
        let _ = kill(pid, Signal::SIGTERM);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("console recorder did not stop in time; killing");
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        if self.output.is_file() {
            let _ = std::fs::remove_dir_all(self.output.with_extension("webm.frames"));
            Some(self.output.clone())
        } else {
            tracing::warn!(
                "no recording produced; see {} for the recorder log",
                self.log.display()
            );
            None
        }
    }
}

impl Drop for ConsoleRecording {
    /// Abnormal-path cleanup (early scenario return): kill Chrome/node
    /// outright rather than leaking them. The graceful `stop()` consumes
    /// `self` before this runs on the normal path.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
