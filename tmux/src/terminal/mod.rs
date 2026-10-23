//! Console terminals: each one is a real tmux client (`tmux attach-session`)
//! running in a PTY this worker owns.
//!
//! The durable state lives in tmux itself, so a terminal is disposable: a
//! page that reloads opens a new client and tmux redraws the whole screen.
//! That is why there are no reconnect tokens here (unlike the ide's
//! `shell::pty::*`, which this deliberately does not use). What must not
//! happen is an orphaned client staying attached — it would keep the
//! session's windows sized to a browser nobody looks at — so a client is
//! detached when output can no longer be delivered, when its page stops
//! sending heartbeats, and on shutdown.
//!
//! Output is pushed fire-and-forget (`Void`) to the page's handler with a
//! per-terminal sequence; the page reorders, asks `replay` for gaps and
//! `redraw` (tmux `refresh-client`) when the buffer no longer has them.

use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction, TriggerAction};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::collector::{Collector, ConsoleClients};
use crate::tmux_cli::{window_target, Tmux, WindowRef};

mod output_buffer;
pub mod protocol;

use output_buffer::OutputBuffer;
use protocol::{
    CloseResponse, OkResponse, OpenRequest, OpenResponse, OutputEvent, Replay, ReplayRequest,
    ResizeRequest, ResizeResponse, SwitchRequest, SwitchResponse, TerminalRef, WriteRequest,
    WriteResponse,
};

const MAX_INPUT_BYTES: usize = 64 * 1024;
const OUTPUT_BUFFER_BYTES: usize = 1024 * 1024;
const OUTPUT_FUNCTION_PREFIX: &str = "iii::tmux-ui::pty-output::console-";
const REAP_EVERY: Duration = Duration::from_secs(5);
const HUP_WAIT: Duration = Duration::from_millis(500);
/// The worker's own identity must not leak into the client (and from there,
/// through `update-environment`, into the session).
const STRIPPED_ENV: [&str; 4] = ["TMUX", "TMUX_PANE", "III_WORKER_NAME", "III_NAMESPACE"];

struct Terminal {
    id: String,
    access_key: String,
    owner: String,
    output_function_id: String,
    client_tty: Option<String>,
    pid: Option<u32>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<OutputBuffer>,
    last_seen: Mutex<Instant>,
}

impl Terminal {
    fn touch(&self) {
        *self.last_seen.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
    }
}

/// Where terminal output goes. The engine in production; a channel in tests.
pub trait OutputSink: Send + Sync + 'static {
    /// Deliver one event; `false` means the page is unreachable.
    fn deliver(
        &self,
        function_id: String,
        event: OutputEvent,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
}

impl OutputSink for IIIClient {
    fn deliver(
        &self,
        function_id: String,
        event: OutputEvent,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { emit(self, &function_id, event).await })
    }
}

#[derive(Clone)]
pub struct TerminalManager {
    sink: Arc<dyn OutputSink>,
    collector: Arc<Collector>,
    console: ConsoleClients,
    terminals: Arc<RwLock<HashMap<String, Arc<Terminal>>>>,
    on_change: Arc<dyn Fn() + Send + Sync>,
    /// The engine's observe support (`IIIClient::engine_supports_observe`),
    /// which decides whether the payload's caller stamp may be read.
    engine_observes: Arc<dyn Fn() -> Option<bool> + Send + Sync>,
}

pub fn validate_output_function_id(function_id: &str) -> Result<(), String> {
    let browser_id = function_id
        .strip_prefix(OUTPUT_FUNCTION_PREFIX)
        .ok_or_else(|| "terminal output function is not a tmux console handler".to_string())?;
    if browser_id.is_empty()
        || browser_id.len() > 128
        || !browser_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("terminal output function has an invalid browser id".to_string());
    }
    Ok(())
}

fn validate_size(cols: u16, rows: u16) -> Result<(), String> {
    if !(2..=500).contains(&cols) || !(2..=500).contains(&rows) {
        return Err("terminal size must be between 2 and 500 columns/rows".to_string());
    }
    Ok(())
}

/// The worker calling a terminal function, as the engine stamped it: the
/// invocation's `caller.worker_id`. The payload's `_caller_worker_id` is read
/// only when `engine_observes` is `Some(false)`: a 0.24 engine sends nothing
/// else, while an engine with observe stamps the caller on the invocation
/// itself, so there the payload field is never trusted. Fails closed without
/// a caller, while the engine's support is unknown, and for an observer
/// activation: terminal calls come straight from the page that opened the
/// terminal, and an activation carries the engine's or an owner's identity,
/// not the page's.
///
/// Reads the task-local invocation, so call it in the handler's own future.
fn require_caller(stamped: Option<&str>, engine_observes: Option<bool>) -> Result<String, String> {
    let invocation = iii_sdk::current_invocation();
    if invocation
        .as_ref()
        .is_some_and(|invocation| invocation.activation.is_some())
    {
        return Err("terminal calls cannot come from an observer activation".to_string());
    }
    invocation
        .and_then(|invocation| invocation.caller?.worker_id)
        .filter(|caller| !caller.is_empty())
        .or_else(|| {
            stamped
                .filter(|caller| engine_observes == Some(false) && !caller.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| "terminal calls require engine-stamped caller identity".to_string())
}

/// `tmux [-L name | -S path] -u attach-session -t <session>`.
pub fn attach_args(tmux: &Tmux, session_id: &str) -> Vec<String> {
    let mut args = tmux.socket.args();
    args.extend(["-u", "attach-session", "-t", session_id].map(str::to_string));
    args
}

struct Spawned {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

fn spawn_client(tmux: &Tmux, session_id: &str, cols: u16, rows: u16) -> anyhow::Result<Spawned> {
    let pair = native_pty_system().openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut command = CommandBuilder::new(&tmux.path);
    command.args(attach_args(tmux, session_id));
    if let Some(home) = std::env::var_os("HOME") {
        command.cwd(home);
    }
    for key in STRIPPED_ENV {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    let child = pair.slave.spawn_command(command)?;
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    drop(pair.slave);
    Ok(Spawned {
        master: pair.master,
        reader,
        writer,
        child,
    })
}

async fn emit(iii: &IIIClient, function_id: &str, event: OutputEvent) -> bool {
    let Ok(payload) = serde_json::to_value(event) else {
        return false;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        })
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::debug!(%error, "terminal output delivery failed");
            false
        }
    }
}

impl TerminalManager {
    pub fn new(
        sink: Arc<dyn OutputSink>,
        collector: Arc<Collector>,
        console: ConsoleClients,
        on_change: Arc<dyn Fn() + Send + Sync>,
        engine_observes: Arc<dyn Fn() -> Option<bool> + Send + Sync>,
    ) -> Self {
        Self {
            sink,
            collector,
            console,
            terminals: Arc::new(RwLock::new(HashMap::new())),
            on_change,
            engine_observes,
        }
    }

    fn caller(&self, stamped: Option<&str>) -> Result<String, String> {
        require_caller(stamped, (self.engine_observes)())
    }

    async fn authenticated(&self, reference: &TerminalRef) -> Result<Arc<Terminal>, String> {
        let caller = self.caller(reference.caller_worker_id.as_deref())?;
        let terminal = self
            .terminals
            .read()
            .await
            .get(&reference.terminal_id)
            .cloned()
            .ok_or_else(|| format!("terminal {} not found", reference.terminal_id))?;
        if terminal.access_key != reference.access_key || terminal.owner != caller {
            return Err("terminal credentials are invalid".to_string());
        }
        terminal.touch();
        Ok(terminal)
    }

    pub async fn open(&self, request: OpenRequest) -> Result<OpenResponse, String> {
        let caller = self.caller(request.caller_worker_id.as_deref())?;
        validate_size(request.cols, request.rows)?;
        validate_output_function_id(&request.output_function_id)?;
        let max = self.collector.config().max_terminals;
        if self.terminals.read().await.len() >= max {
            return Err(format!("at most {max} console terminals can be open"));
        }
        let tmux = self.collector.tmux().await?;
        let session_id = tmux.resolve_session(&request.session).await?;
        if let Some(window) = &request.window {
            let window = WindowRef::parse(window)?;
            let window_id = tmux.resolve_window(&session_id, &window).await?;
            tmux.run([
                "select-window",
                "-t",
                &window_target(&session_id, &WindowRef::Id(window_id)),
            ])
            .await
            .map_err(|error| error.to_string())?;
        }

        let (cols, rows) = (request.cols, request.rows);
        let spawn_tmux = tmux.clone();
        let spawn_session = session_id.clone();
        let spawned = tokio::task::spawn_blocking(move || {
            spawn_client(&spawn_tmux, &spawn_session, cols, rows)
        })
        .await
        .map_err(|error| format!("terminal spawn task failed: {error}"))?
        .map_err(|error| format!("could not start tmux attach: {error}"))?;

        let client_tty = spawned
            .master
            .tty_name()
            .map(|path| path.to_string_lossy().into_owned());
        let terminal = Arc::new(Terminal {
            id: Uuid::new_v4().simple().to_string(),
            access_key: Uuid::new_v4().simple().to_string(),
            owner: caller,
            output_function_id: request.output_function_id,
            client_tty: client_tty.clone(),
            pid: spawned.child.process_id(),
            master: Mutex::new(spawned.master),
            writer: Mutex::new(spawned.writer),
            killer: Mutex::new(spawned.child.clone_killer()),
            output: Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_BYTES)),
            last_seen: Mutex::new(Instant::now()),
        });
        if let Some(tty) = &client_tty {
            self.console.insert(tty);
        }
        self.terminals
            .write()
            .await
            .insert(terminal.id.clone(), terminal.clone());
        self.spawn_pump(terminal.clone(), spawned.reader, spawned.child);
        (self.on_change)();
        tracing::info!(terminal = %terminal.id, session = %session_id, tty = ?client_tty, "console terminal attached");
        Ok(OpenResponse {
            terminal_id: terminal.id.clone(),
            access_key: terminal.access_key.clone(),
            session_id,
            client_tty,
        })
    }

    fn spawn_pump(
        &self,
        terminal: Arc<Terminal>,
        mut reader: Box<dyn Read + Send>,
        mut child: Box<dyn Child + Send + Sync>,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        tokio::task::spawn_blocking(move || {
            let mut chunk = vec![0_u8; 16 * 1024];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if tx.blocking_send(chunk[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let child_wait = tokio::task::spawn_blocking(move || child.wait());
        let manager = self.clone();
        tokio::spawn(async move {
            let mut delivered = true;
            while let Some(data) = rx.recv().await {
                let frame = terminal
                    .output
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(data);
                if !delivered {
                    continue;
                }
                delivered = manager
                    .sink
                    .deliver(
                        terminal.output_function_id.clone(),
                        OutputEvent {
                            terminal_id: terminal.id.clone(),
                            sequence: Some(frame.sequence),
                            data: Some(frame.data),
                            eof: false,
                            exit_code: None,
                            error: None,
                        },
                    )
                    .await;
                if !delivered {
                    // The page is gone: detach its client so it stops
                    // holding the session's size.
                    tracing::info!(terminal = %terminal.id, "console terminal unreachable; detaching");
                    let manager = manager.clone();
                    let id = terminal.id.clone();
                    tokio::spawn(async move {
                        manager.terminate(&id).await;
                    });
                }
            }
            let (exit_code, error) = match child_wait.await {
                Ok(Ok(status)) => (Some(status.exit_code()), None),
                Ok(Err(error)) => (None, Some(error.to_string())),
                Err(error) => (None, Some(error.to_string())),
            };
            manager.forget(&terminal.id).await;
            if delivered {
                manager
                    .sink
                    .deliver(
                        terminal.output_function_id.clone(),
                        OutputEvent {
                            terminal_id: terminal.id.clone(),
                            sequence: None,
                            data: None,
                            eof: true,
                            exit_code,
                            error,
                        },
                    )
                    .await;
            }
            tracing::info!(terminal = %terminal.id, ?exit_code, "console terminal detached");
        });
    }

    /// Drop a terminal from the registry (its client already exited).
    async fn forget(&self, id: &str) {
        let removed = self.terminals.write().await.remove(id);
        if let Some(terminal) = removed {
            if let Some(tty) = &terminal.client_tty {
                self.console.remove(tty);
            }
            (self.on_change)();
        }
    }

    /// Detach the client: SIGHUP (tmux detaches cleanly), then kill if it lingers.
    async fn terminate(&self, id: &str) -> bool {
        let Some(terminal) = self.terminals.read().await.get(id).cloned() else {
            return false;
        };
        let pid = terminal.pid;
        let waited = tokio::task::spawn_blocking(move || {
            if let Some(pid) = pid
                .and_then(|pid| libc::pid_t::try_from(pid).ok())
                .filter(|pid| *pid > 1)
            {
                // SAFETY: plain kill(2) on the client pid this worker spawned.
                unsafe {
                    libc::kill(pid, libc::SIGHUP);
                }
                let deadline = Instant::now() + HUP_WAIT;
                while Instant::now() < deadline {
                    // SAFETY: signal 0 only probes for existence.
                    if unsafe { libc::kill(pid, 0) } != 0 {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let _ = terminal
                .killer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .kill();
            false
        })
        .await;
        if waited.is_err() {
            tracing::warn!(terminal = %id, "terminal close task failed");
        }
        self.forget(id).await;
        true
    }

    pub async fn write(&self, request: WriteRequest) -> Result<WriteResponse, String> {
        let terminal = self.authenticated(&request.terminal).await?;
        let data = BASE64_STANDARD
            .decode(request.data.as_bytes())
            .map_err(|error| format!("terminal input is not base64: {error}"))?;
        if data.len() > MAX_INPUT_BYTES {
            return Err(format!("terminal input exceeds {MAX_INPUT_BYTES} bytes"));
        }
        let written = data.len();
        tokio::task::spawn_blocking(move || {
            let mut writer = terminal.writer.lock().unwrap_or_else(|p| p.into_inner());
            writer.write_all(&data)?;
            writer.flush()
        })
        .await
        .map_err(|error| format!("terminal write task failed: {error}"))?
        .map_err(|error| format!("terminal write failed: {error}"))?;
        Ok(WriteResponse { written })
    }

    pub async fn resize(&self, request: ResizeRequest) -> Result<ResizeResponse, String> {
        validate_size(request.cols, request.rows)?;
        let terminal = self.authenticated(&request.terminal).await?;
        let (cols, rows) = (request.cols, request.rows);
        tokio::task::spawn_blocking(move || {
            terminal
                .master
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
        })
        .await
        .map_err(|error| format!("terminal resize task failed: {error}"))?
        .map_err(|error| format!("terminal resize failed: {error}"))?;
        Ok(ResizeResponse { cols, rows })
    }

    fn client_tty(terminal: &Terminal) -> Result<&str, String> {
        terminal
            .client_tty
            .as_deref()
            .ok_or_else(|| "this terminal's tmux client has no known tty".to_string())
    }

    pub async fn switch(&self, request: SwitchRequest) -> Result<SwitchResponse, String> {
        let terminal = self.authenticated(&request.terminal).await?;
        let tty = Self::client_tty(&terminal)?;
        let tmux = self.collector.tmux().await?;
        let session_id = tmux.resolve_session(&request.session).await?;
        let window_id = match &request.window {
            Some(window) => Some(
                tmux.resolve_window(&session_id, &WindowRef::parse(window)?)
                    .await?,
            ),
            None => None,
        };
        tmux.run(["switch-client", "-c", tty, "-t", &session_id])
            .await
            .map_err(|error| error.to_string())?;
        if let Some(window_id) = window_id {
            tmux.run([
                "select-window",
                "-t",
                &window_target(&session_id, &WindowRef::Id(window_id)),
            ])
            .await
            .map_err(|error| error.to_string())?;
        }
        (self.on_change)();
        Ok(SwitchResponse { session_id })
    }

    pub async fn replay(&self, request: ReplayRequest) -> Result<Replay, String> {
        let terminal = self.authenticated(&request.terminal).await?;
        let replay = terminal
            .output
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .frames_after(request.after_sequence);
        Ok(replay)
    }

    pub async fn redraw(&self, request: TerminalRef) -> Result<OkResponse, String> {
        let terminal = self.authenticated(&request).await?;
        let tty = Self::client_tty(&terminal)?;
        let tmux = self.collector.tmux().await?;
        tmux.run(["refresh-client", "-t", tty])
            .await
            .map_err(|error| error.to_string())?;
        Ok(OkResponse { ok: true })
    }

    pub async fn close(&self, request: TerminalRef) -> Result<CloseResponse, String> {
        let caller = self.caller(request.caller_worker_id.as_deref())?;
        let terminal = self
            .terminals
            .read()
            .await
            .get(&request.terminal_id)
            .cloned();
        let Some(terminal) = terminal else {
            return Ok(CloseResponse { closed: false });
        };
        if terminal.access_key != request.access_key || terminal.owner != caller {
            return Err("terminal credentials are invalid".to_string());
        }
        Ok(CloseResponse {
            closed: self.terminate(&terminal.id).await,
        })
    }

    pub async fn close_all(&self) -> usize {
        let ids: Vec<String> = self.terminals.read().await.keys().cloned().collect();
        for id in &ids {
            self.terminate(id).await;
        }
        ids.len()
    }

    pub async fn count(&self) -> usize {
        self.terminals.read().await.len()
    }

    /// Detach every terminal without input or heartbeat for longer than `timeout`.
    pub async fn reap_idle(&self, timeout: Duration) -> usize {
        let stale: Vec<String> = self
            .terminals
            .read()
            .await
            .values()
            .filter(|terminal| terminal.idle_for() > timeout)
            .map(|terminal| terminal.id.clone())
            .collect();
        for id in &stale {
            tracing::info!(terminal = %id, "console terminal idle; detaching");
            self.terminate(id).await;
        }
        stale.len()
    }

    /// Detach terminals whose page stopped sending heartbeats.
    pub fn spawn_reaper(&self) {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(REAP_EVERY);
            loop {
                tick.tick().await;
                let timeout =
                    Duration::from_millis(manager.collector.config().terminal_idle_timeout_ms);
                manager.reap_idle(timeout).await;
            }
        });
    }
}

macro_rules! register_terminal_fn {
    ($iii:expr, $manager:expr, $id:expr, $desc:expr, $req:ty, $method:ident) => {{
        let manager = $manager.clone();
        $iii.register_function(
            $id,
            RegisterFunction::new_async(move |request: $req| {
                let manager = manager.clone();
                async move { manager.$method(request).await.map_err(Error::Handler) }
            })
            .description($desc)
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }};
}

pub const FUNCTIONS: [(&str, &str); 7] = [
    ("tmux::terminal::open", "Attach a console terminal to a tmux session as a real tmux client running in a PTY. Output is pushed to the page's handler."),
    ("tmux::terminal::write", "Write base64 keyboard input to a console terminal's tmux client."),
    ("tmux::terminal::resize", "Resize a console terminal; also its heartbeat."),
    ("tmux::terminal::switch", "Move only this console terminal's tmux client to another session (switch-client), optionally selecting a window there."),
    ("tmux::terminal::replay", "Re-send buffered output frames after a sequence, for gap repair."),
    ("tmux::terminal::redraw", "Ask tmux to repaint the console terminal's whole screen (refresh-client)."),
    ("tmux::terminal::close", "Detach a console terminal's tmux client. The session keeps running."),
];

pub fn register(iii: &IIIClient, manager: TerminalManager) {
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[0].0,
        FUNCTIONS[0].1,
        OpenRequest,
        open
    );
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[1].0,
        FUNCTIONS[1].1,
        WriteRequest,
        write
    );
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[2].0,
        FUNCTIONS[2].1,
        ResizeRequest,
        resize
    );
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[3].0,
        FUNCTIONS[3].1,
        SwitchRequest,
        switch
    );
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[4].0,
        FUNCTIONS[4].1,
        ReplayRequest,
        replay
    );
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[5].0,
        FUNCTIONS[5].1,
        TerminalRef,
        redraw
    );
    register_terminal_fn!(
        iii,
        manager,
        FUNCTIONS[6].0,
        FUNCTIONS[6].1,
        TerminalRef,
        close
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux_cli::Socket;
    use iii_sdk::{scope, Activation, Caller, InvocationCtx};

    #[test]
    fn output_target_is_this_pages_console_handler() {
        validate_output_function_id("iii::tmux-ui::pty-output::console-1b2c-3d").unwrap();
        assert!(validate_output_function_id("iii::shell-ui::pty-output::console-1").is_err());
        assert!(validate_output_function_id("tmux::sessions::kill").is_err());
        assert!(validate_output_function_id("iii::tmux-ui::pty-output::console-").is_err());
        assert!(validate_output_function_id("iii::tmux-ui::pty-output::console-a::b").is_err());
    }

    #[test]
    fn sizes_are_bounded() {
        validate_size(80, 24).unwrap();
        assert!(validate_size(1, 24).is_err());
        assert!(validate_size(80, 501).is_err());
    }

    fn invocation(caller: Option<Caller>, activation: Option<Activation>) -> InvocationCtx {
        InvocationCtx {
            function_id: "tmux::terminal::write".into(),
            invocation_id: "inv-1".into(),
            caller,
            activation,
            observe_depth: 0,
            metadata: None,
            traceparent: None,
            baggage: None,
        }
    }

    fn stamped_caller(worker_id: &str) -> Option<Caller> {
        Some(Caller {
            worker_id: Some(worker_id.into()),
            ..Caller::default()
        })
    }

    const ENGINES: [Option<bool>; 3] = [Some(true), Some(false), None];

    async fn caller_in(
        ctx: InvocationCtx,
        stamped: Option<&str>,
        engine_observes: Option<bool>,
    ) -> Result<String, String> {
        scope(ctx, async { require_caller(stamped, engine_observes) }).await
    }

    #[tokio::test]
    async fn invocation_caller_wins_over_the_payload_stamp() {
        for engine in ENGINES {
            for stamped in [Some("forged"), Some(""), None] {
                assert_eq!(
                    caller_in(invocation(stamped_caller("page"), None), stamped, engine).await,
                    Ok("page".to_string())
                );
            }
        }
    }

    #[tokio::test]
    async fn payload_stamp_serves_engines_without_observe() {
        // A 0.24 engine sends no caller, or one without a worker id.
        for caller in [None, stamped_caller(""), Some(Caller::default())] {
            assert_eq!(
                caller_in(invocation(caller, None), Some("page"), Some(false)).await,
                Ok("page".to_string())
            );
        }
        // Outside any invocation.
        assert_eq!(
            require_caller(Some("page"), Some(false)),
            Ok("page".to_string())
        );
    }

    #[tokio::test]
    async fn payload_stamp_is_never_read_unless_the_engine_lacks_observe() {
        // An engine with observe stamps the caller on the invocation; until
        // the engine's support is known, nothing is trusted either.
        for engine in [Some(true), None] {
            for caller in [None, stamped_caller(""), Some(Caller::default())] {
                assert!(caller_in(invocation(caller, None), Some("page"), engine)
                    .await
                    .unwrap_err()
                    .contains("require engine-stamped caller identity"));
            }
            assert!(require_caller(Some("page"), engine).is_err());
        }
    }

    #[tokio::test]
    async fn no_caller_at_all_fails_closed() {
        for engine in ENGINES {
            for stamped in [None, Some("")] {
                for result in [
                    require_caller(stamped, engine),
                    caller_in(invocation(None, None), stamped, engine).await,
                    caller_in(invocation(stamped_caller(""), None), stamped, engine).await,
                ] {
                    assert!(result
                        .unwrap_err()
                        .contains("require engine-stamped caller identity"));
                }
            }
        }
    }

    #[tokio::test]
    async fn observer_activations_are_refused() {
        let activation = Activation {
            observer_id: "obs-1".into(),
            activation_id: Some("act-1".into()),
        };
        for engine in ENGINES {
            for (caller, stamped) in [
                (stamped_caller("page"), Some("page")),
                (None, Some("page")),
                (None, None),
            ] {
                let refused = caller_in(
                    invocation(caller, Some(activation.clone())),
                    stamped,
                    engine,
                )
                .await
                .unwrap_err();
                assert!(refused.contains("observer activation"));
            }
        }
    }

    #[test]
    fn attach_uses_the_configured_socket() {
        let tmux = Tmux {
            path: "/usr/bin/tmux".into(),
            socket: Socket::Name("work".into()),
            timeout: Duration::from_secs(1),
        };
        assert_eq!(
            attach_args(&tmux, "$3"),
            vec!["-L", "work", "-u", "attach-session", "-t", "$3"]
        );
    }
}
