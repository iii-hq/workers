use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
#[cfg(unix)]
use std::time::Instant;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction, TriggerAction};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
#[cfg(test)]
use tokio::sync::Notify;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, RwLock, Semaphore};
use uuid::Uuid;

use crate::code::state::ResolverCell;

mod output_buffer;
mod protocol;
mod session;

#[cfg(test)]
use output_buffer::OutputFrame;
use output_buffer::{OutputBuffer, MAX_OUTPUT_BUFFER_BYTES};
pub use protocol::{
    AdoptRequest, AttachRequest, AttachResponse, CloseRequest, CloseResponse, DetachRequest,
    DetachResponse, OpenRequest, OpenResponse, ResizeRequest, ResizeResponse, SessionSummary,
    SessionsRequest, SessionsResponse, WriteRequest, WriteResponse,
};
use session::{SessionControl, SessionStatus};

const MAX_SESSIONS: usize = 16;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const OUTPUT_FUNCTION_INFIX: &str = "::pty-output::console-";
#[cfg(test)]
const OUTPUT_FUNCTION_PREFIX: &str = "iii::shell-ui::pty-output::console-";
const TERMINATION_POLL_INTERVAL: StdDuration = StdDuration::from_millis(10);
const PORTABLE_TERMINATION_WAIT: StdDuration = StdDuration::from_secs(2);
#[cfg(unix)]
const SIGHUP_TERMINATION_WAIT: StdDuration = StdDuration::from_millis(150);
#[cfg(unix)]
const SIGTERM_TERMINATION_WAIT: StdDuration = StdDuration::from_millis(250);
#[cfg(unix)]
const SIGKILL_TERMINATION_WAIT: StdDuration = StdDuration::from_secs(2);

#[derive(Clone)]
pub struct PtyManager {
    iii: IIIClient,
    resolver: ResolverCell,
    sessions: Arc<RwLock<HashMap<String, Arc<PtySession>>>>,
    permits: Arc<Semaphore>,
    open_requests: Arc<AsyncMutex<HashMap<String, OpenResponse>>>,
    shutdown: Arc<RwLock<bool>>,
    program: Option<Arc<str>>,
    #[cfg(test)]
    open_pause: Option<Arc<OpenPause>>,
}

#[cfg(test)]
struct OpenPause {
    reached: Notify,
    release: Notify,
}

struct PtySession {
    control: AsyncMutex<SessionControl>,
    lifecycle: AsyncMutex<SessionLifecycle>,
    write_serial: AsyncMutex<()>,
    output: Mutex<OutputBuffer>,
    status: Mutex<SessionStatus>,
    cwd: String,
    program: Option<String>,
    pid: Option<u32>,
    #[cfg(unix)]
    process_group: Mutex<Option<libc::pid_t>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    permit: Mutex<Option<OwnedSemaphorePermit>>,
}

#[derive(Default)]
struct SessionLifecycle {
    closing: bool,
    closed: bool,
}

struct SpawnedPty {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Debug, Serialize)]
struct OutputEvent {
    session_id: String,
    sequence: Option<u64>,
    data: Option<String>,
    eof: bool,
    exit_code: Option<u32>,
    signal: Option<String>,
    error: Option<String>,
}

impl PtyManager {
    pub fn new(iii: IIIClient, resolver: ResolverCell) -> Self {
        Self {
            iii,
            resolver,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            permits: Arc::new(Semaphore::new(MAX_SESSIONS)),
            open_requests: Arc::new(AsyncMutex::new(HashMap::new())),
            shutdown: Arc::new(RwLock::new(false)),
            program: None,
            #[cfg(test)]
            open_pause: None,
        }
    }

    #[cfg(test)]
    fn with_program(mut self, program: &str) -> Self {
        self.program = Some(Arc::from(program));
        self
    }

    pub async fn open(&self, req: OpenRequest) -> Result<OpenResponse, String> {
        validate_size(req.cols, req.rows)?;
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?.to_string();
        let request_key = req
            .request_id
            .as_deref()
            .map(validate_request_id)
            .transpose()?
            .map(|request_id| format!("{caller_worker_id}:{request_id}"));
        let mut open_requests = self.open_requests.lock().await;
        if let Some(cached) = request_key.as_ref().and_then(|key| open_requests.get(key)) {
            return Ok(cached.clone());
        }
        validate_output_function_id(&req.output_function_id)?;
        let shutdown = self.shutdown.read().await;
        if *shutdown {
            return Err("terminal manager is shutting down".to_string());
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| format!("terminal session limit reached ({MAX_SESSIONS})"))?;

        let resolver = self.resolver.read().await.clone();
        let cwd = resolver
            .resolve(&req.cwd)
            .map_err(|error| format!("terminal cwd is outside the shell workspace: {error}"))?;
        if !cwd.is_dir() {
            return Err(format!(
                "terminal cwd is not a directory: {}",
                cwd.display()
            ));
        }

        let spawn_cwd = cwd.clone();
        let cols = req.cols;
        let rows = req.rows;
        // The configured program (tests, and any future policy) wins over the
        // request, so a fixed-program deployment stays fixed.
        let program = match &self.program {
            Some(program) => Some(program.to_string()),
            None => req.program.clone().filter(|program| !program.is_empty()),
        };
        let args = req.args.clone().unwrap_or_default();
        let env = req.env.clone().unwrap_or_default();
        reject_dangerous_env(&env)?;
        let spawn_program = program.clone();
        let spawned = tokio::task::spawn_blocking(move || {
            spawn_process(
                &spawn_cwd,
                cols,
                rows,
                spawn_program.as_deref(),
                &args,
                &env,
            )
        })
        .await
        .map_err(|error| format!("terminal spawn task failed: {error}"))?
        .map_err(|error| format!("terminal spawn failed: {error}"))?;

        let session_id = Uuid::new_v4().to_string();
        let control = SessionControl::new(&caller_worker_id, &req.output_function_id);
        let access_key = control.access_key().to_string();
        let reconnect_token = control.reconnect_token().to_string();
        let pid = spawned.child.process_id();
        #[cfg(unix)]
        let process_group = spawned.master.process_group_leader();
        let killer = spawned.child.clone_killer();
        let SpawnedPty {
            master,
            reader,
            writer,
            child,
        } = spawned;
        let session = Arc::new(PtySession {
            control: AsyncMutex::new(control),
            lifecycle: AsyncMutex::new(SessionLifecycle::default()),
            write_serial: AsyncMutex::new(()),
            output: Mutex::new(OutputBuffer::new(MAX_OUTPUT_BUFFER_BYTES)),
            status: Mutex::new(SessionStatus::Attached),
            cwd: cwd.display().to_string(),
            program: program.clone(),
            pid,
            #[cfg(unix)]
            process_group: Mutex::new(process_group),
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            permit: Mutex::new(Some(permit)),
        });
        #[cfg(test)]
        if let Some(pause) = &self.open_pause {
            pause.reached.notify_one();
            pause.release.notified().await;
        }
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), session);
        self.spawn_output_pump(session_id.clone(), reader, child);

        let response = OpenResponse {
            session_id,
            access_key,
            reconnect_token,
            pid,
            cwd: cwd.display().to_string(),
            program,
        };
        if let Some(request_key) = request_key {
            open_requests.insert(request_key, response.clone());
        }
        Ok(response)
    }

    pub async fn write(&self, req: WriteRequest) -> Result<WriteResponse, String> {
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?;
        let data = BASE64_STANDARD
            .decode(&req.data)
            .map_err(|error| format!("terminal input is not valid base64: {error}"))?;
        if data.len() > MAX_INPUT_BYTES {
            return Err(format!("terminal input exceeds {MAX_INPUT_BYTES} bytes"));
        }
        let session = self.session(&req.session_id).await?;
        {
            let lifecycle = session.lifecycle.lock().await;
            ensure_session_open(&lifecycle)?;
            authenticate_session(&session, &req.access_key, caller_worker_id).await?;
        }
        let _write_guard = session.write_serial.lock().await;
        {
            let lifecycle = session.lifecycle.lock().await;
            ensure_session_open(&lifecycle)?;
            authenticate_session(&session, &req.access_key, caller_worker_id).await?;
        }
        let written = data.len();
        let write_session = session.clone();
        let write_result = tokio::task::spawn_blocking(move || {
            let mut writer = write_session
                .writer
                .lock()
                .map_err(|_| "terminal writer lock poisoned".to_string())?;
            writer
                .write_all(&data)
                .and_then(|_| writer.flush())
                .map_err(|error| format!("terminal input failed: {error}"))
        })
        .await
        .map_err(|error| format!("terminal input task failed: {error}"))?;
        if let Err(error) = write_result {
            let lifecycle = session.lifecycle.lock().await;
            ensure_session_open(&lifecycle)?;
            return Err(error);
        }
        let lifecycle = session.lifecycle.lock().await;
        ensure_session_open(&lifecycle)?;
        Ok(WriteResponse { written })
    }

    pub async fn resize(&self, req: ResizeRequest) -> Result<ResizeResponse, String> {
        validate_size(req.cols, req.rows)?;
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?;
        let session = self.session(&req.session_id).await?;
        let lifecycle = session.lifecycle.lock().await;
        ensure_session_open(&lifecycle)?;
        authenticate_session(&session, &req.access_key, caller_worker_id).await?;
        let cols = req.cols;
        let rows = req.rows;
        resize_session(session.clone(), cols, rows).await?;
        drop(lifecycle);
        Ok(ResizeResponse { cols, rows })
    }

    pub async fn close(&self, req: CloseRequest) -> Result<CloseResponse, String> {
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?;
        let session = {
            let sessions = self.sessions.read().await;
            sessions.get(&req.session_id).cloned()
        };
        let Some(session) = session else {
            return Ok(CloseResponse { closed: false });
        };
        let mut lifecycle = session.lifecycle.lock().await;
        ensure_session_open(&lifecycle)?;
        authenticate_session(&session, &req.access_key, caller_worker_id).await?;
        lifecycle.closing = true;
        drop(lifecycle);
        if let Err(error) = terminate_session(session.clone()).await {
            reset_session_closing(&session).await;
            return Err(error);
        }
        if let Err(error) = release_session_permit(&session) {
            reset_session_closing(&session).await;
            return Err(error);
        }
        {
            let mut lifecycle = session.lifecycle.lock().await;
            lifecycle.closing = false;
            lifecycle.closed = true;
        }
        remove_session(&self.sessions, &req.session_id, &session).await;
        self.open_requests
            .lock()
            .await
            .retain(|_, response| response.session_id != req.session_id);
        Ok(CloseResponse { closed: true })
    }

    pub async fn close_all(&self) -> Result<usize, String> {
        let mut shutdown = self.shutdown.write().await;
        *shutdown = true;
        let sessions = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .map(|(session_id, session)| (session_id.clone(), session.clone()))
                .collect::<Vec<_>>()
        };
        let mut closed = 0;
        let mut failures = Vec::new();
        for (session_id, session) in sessions {
            let mut lifecycle = session.lifecycle.lock().await;
            if lifecycle.closed {
                continue;
            }
            if lifecycle.closing {
                failures.push(format!("{session_id}: terminal session is already closing"));
                continue;
            }
            lifecycle.closing = true;
            drop(lifecycle);
            if let Err(error) = terminate_session(session.clone()).await {
                reset_session_closing(&session).await;
                failures.push(format!("{session_id}: {error}"));
                continue;
            }
            if let Err(error) = release_session_permit(&session) {
                reset_session_closing(&session).await;
                failures.push(format!("{session_id}: {error}"));
                continue;
            }
            {
                let mut lifecycle = session.lifecycle.lock().await;
                lifecycle.closing = false;
                lifecycle.closed = true;
            }
            remove_session(&self.sessions, &session_id, &session).await;
            closed += 1;
        }
        if failures.is_empty() {
            self.open_requests.lock().await.clear();
            Ok(closed)
        } else {
            Err(format!(
                "failed to close {} terminal session(s): {}",
                failures.len(),
                failures.join("; ")
            ))
        }
    }

    pub async fn detach(&self, req: DetachRequest) -> Result<DetachResponse, String> {
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?;
        let session = self.session(&req.session_id).await?;
        let lifecycle = session.lifecycle.lock().await;
        ensure_session_open(&lifecycle)?;
        let mut control = session.control.lock().await;
        control.detach(&req.access_key, caller_worker_id)?;
        let mut status = session
            .status
            .lock()
            .map_err(|_| "terminal status lock poisoned".to_string())?;
        if !matches!(*status, SessionStatus::Exited { .. }) {
            *status = SessionStatus::Detached;
        }
        Ok(DetachResponse {
            status: status.clone(),
        })
    }

    pub async fn attach(&self, req: AttachRequest) -> Result<AttachResponse, String> {
        validate_size(req.cols, req.rows)?;
        validate_output_function_id(&req.output_function_id)?;
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?;
        let request_id = req
            .request_id
            .as_deref()
            .map(validate_request_id)
            .transpose()?;
        let session = self.session(&req.session_id).await?;
        let lifecycle = session.lifecycle.lock().await;
        ensure_session_open(&lifecycle)?;

        {
            let control = session.control.lock().await;
            if !control.can_attach(&req.reconnect_token, request_id) {
                return Err("terminal reconnect token is invalid".to_string());
            }
        }

        let cols = req.cols;
        let rows = req.rows;
        resize_session(session.clone(), cols, rows).await?;

        let mut control = session.control.lock().await;
        if !control.can_attach(&req.reconnect_token, request_id) {
            return Err("terminal reconnect token is invalid".to_string());
        }
        let replay = session
            .output
            .lock()
            .map_err(|_| "terminal output lock poisoned".to_string())?
            .frames_after(req.after_sequence);
        let credentials = control.attach(
            &req.reconnect_token,
            request_id,
            caller_worker_id,
            &req.output_function_id,
        )?;
        let mut status = session
            .status
            .lock()
            .map_err(|_| "terminal status lock poisoned".to_string())?;
        if !matches!(*status, SessionStatus::Exited { .. }) {
            *status = SessionStatus::Attached;
        }
        let status = status.clone();

        Ok(AttachResponse {
            access_key: credentials.access_key,
            reconnect_token: credentials.reconnect_token,
            frames: replay.frames,
            truncated: replay.truncated,
            next_sequence: replay.next_sequence,
            cwd: session.cwd.clone(),
            status,
        })
    }

    /// Take back a session whose reconnect token is gone.
    ///
    /// A browser that loses its storage loses the token, and the program keeps
    /// running with nobody able to reach it — an agent still working in a
    /// workspace, invisible. Adoption is the way back, under two rules that
    /// keep it from being a way in:
    ///
    /// 1. The session must be unattached. A live viewer's terminal can never
    ///    be taken; only one nobody is holding.
    /// 2. The new output handler must name the same console page as the old
    ///    one, so the claude page adopts claude sessions and nothing else. The
    ///    browser id may differ — that is the whole point — but the page may
    ///    not.
    ///
    /// Credentials rotate, so whatever the previous owner still held is dead.
    pub async fn adopt(&self, req: AdoptRequest) -> Result<AttachResponse, String> {
        validate_size(req.cols, req.rows)?;
        validate_output_function_id(&req.output_function_id)?;
        let caller_worker_id = require_caller(req.caller_worker_id.as_deref())?;
        let session = self.session(&req.session_id).await?;
        let lifecycle = session.lifecycle.lock().await;
        ensure_session_open(&lifecycle)?;

        {
            let control = session.control.lock().await;
            let wanted = output_ui_name(&req.output_function_id);
            let held = output_ui_name(control.output_function_id());
            if wanted.is_none() || wanted != held {
                return Err("terminal session belongs to another console page".to_string());
            }
        }

        resize_session(session.clone(), req.cols, req.rows).await?;

        let mut control = session.control.lock().await;
        let replay = session
            .output
            .lock()
            .map_err(|_| "terminal output lock poisoned".to_string())?
            .frames_after(req.after_sequence);
        let credentials = control.adopt(caller_worker_id, &req.output_function_id)?;
        let mut status = session
            .status
            .lock()
            .map_err(|_| "terminal status lock poisoned".to_string())?;
        if !matches!(*status, SessionStatus::Exited { .. }) {
            *status = SessionStatus::Attached;
        }
        let status = status.clone();

        Ok(AttachResponse {
            access_key: credentials.access_key,
            reconnect_token: credentials.reconnect_token,
            frames: replay.frames,
            truncated: replay.truncated,
            next_sequence: replay.next_sequence,
            cwd: session.cwd.clone(),
            status,
        })
    }

    /// Read-only view of what the worker holds, for diagnosing a terminal
    /// that shows nothing: how far the sequence got, how much is still
    /// replayable, and where output is going. No credentials.
    pub async fn sessions(&self) -> SessionsResponse {
        let sessions = self.sessions.read().await.clone();
        let mut summaries = Vec::with_capacity(sessions.len());
        for (session_id, session) in sessions {
            let stats = match session.output.lock() {
                Ok(output) => output.stats(),
                Err(_) => continue,
            };
            let control = session.control.lock().await;
            let attached = control.status() == SessionStatus::Attached;
            summaries.push(SessionSummary {
                session_id,
                cwd: session.cwd.clone(),
                program: session.program.clone(),
                pid: session.pid,
                status: control.status(),
                sequence: stats.sequence,
                frames: stats.frames,
                frame_bytes: stats.frame_bytes,
                truncated: stats.truncated,
                output_function_id: attached.then(|| control.output_function_id().to_string()),
                ui: output_ui_name(control.output_function_id()).map(str::to_string),
            });
        }
        summaries.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        SessionsResponse {
            sessions: summaries,
        }
    }

    async fn session(&self, session_id: &str) -> Result<Arc<PtySession>, String> {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| "terminal session does not exist".to_string())
    }

    fn spawn_output_pump(
        &self,
        session_id: String,
        mut reader: Box<dyn Read + Send>,
        mut child: Box<dyn Child + Send + Sync>,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, String>>(64);
        tokio::task::spawn_blocking(move || {
            let mut chunk = vec![0_u8; 16 * 1024];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        if tx.blocking_send(Ok(chunk[..read].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.blocking_send(Err(error.to_string()));
                        break;
                    }
                }
            }
        });
        let child_wait = tokio::task::spawn_blocking(move || child.wait());
        let iii = self.iii.clone();
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            let mut read_error = None;
            while let Some(chunk) = rx.recv().await {
                match chunk {
                    Ok(data) => {
                        let session = {
                            let sessions = sessions.read().await;
                            sessions.get(&session_id).cloned()
                        };
                        let Some(session) = session else {
                            rx.close();
                            break;
                        };
                        let frame = match session.output.lock() {
                            Ok(mut output) => output.push(data),
                            Err(_) => {
                                read_error = Some("terminal output lock poisoned".to_string());
                                break;
                            }
                        };
                        if let Some((output_function_id, access_key)) =
                            output_target(&session).await
                        {
                            let delivered = emit_output(
                                &iii,
                                &output_function_id,
                                OutputEvent {
                                    session_id: session_id.clone(),
                                    sequence: Some(frame.sequence),
                                    data: Some(frame.data),
                                    eof: false,
                                    exit_code: None,
                                    signal: None,
                                    error: None,
                                },
                            )
                            .await;
                            if !delivered {
                                detach_failed_output(&session, &output_function_id, &access_key)
                                    .await;
                            }
                        }
                    }
                    Err(error) => {
                        read_error = Some(error);
                        break;
                    }
                }
            }
            let (exit_code, signal, wait_error) = match child_wait.await {
                Ok(Ok(status)) => (
                    Some(status.exit_code()),
                    status.signal().map(str::to_string),
                    None,
                ),
                Ok(Err(error)) => (None, None, Some(error.to_string())),
                Err(error) => (None, None, Some(error.to_string())),
            };
            let session = {
                let sessions = sessions.read().await;
                sessions.get(&session_id).cloned()
            };
            if let Some(session) = session {
                let error = read_error.or(wait_error);
                if let Ok(mut status) = session.status.lock() {
                    *status = SessionStatus::Exited {
                        exit_code,
                        signal: signal.clone(),
                        error: error.clone(),
                    };
                }
                #[cfg(unix)]
                clear_terminated_process_group(&session);
                if let Some((output_function_id, access_key)) = output_target(&session).await {
                    let delivered = emit_output(
                        &iii,
                        &output_function_id,
                        OutputEvent {
                            session_id,
                            sequence: None,
                            data: None,
                            eof: true,
                            exit_code,
                            signal,
                            error,
                        },
                    )
                    .await;
                    if !delivered {
                        detach_failed_output(&session, &output_function_id, &access_key).await;
                    }
                }
            }
        });
    }
}

async fn output_target(session: &PtySession) -> Option<(String, String)> {
    let control = session.control.lock().await;
    (control.status() == SessionStatus::Attached).then(|| {
        (
            control.output_function_id().to_string(),
            control.access_key().to_string(),
        )
    })
}

async fn detach_failed_output(session: &PtySession, output_function_id: &str, access_key: &str) {
    let detached = session
        .control
        .lock()
        .await
        .detach_output_target(output_function_id, access_key);
    if detached {
        if let Ok(mut status) = session.status.lock() {
            if !matches!(*status, SessionStatus::Exited { .. }) {
                *status = SessionStatus::Detached;
            }
        }
    }
}

pub fn register(iii: &IIIClient, manager: PtyManager) {
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::open",
            RegisterFunction::new_async(move |req: OpenRequest| {
                let manager = manager.clone();
                async move { manager.open(req).await.map_err(Error::Handler) }
            })
            .description("Open a persistent host PTY running the user's login shell, or the program named in `program`.")
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::write",
            RegisterFunction::new_async(move |req: WriteRequest| {
                let manager = manager.clone();
                async move { manager.write(req).await.map_err(Error::Handler) }
            })
            .description("Write base64-encoded keyboard input to a PTY session.")
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::resize",
            RegisterFunction::new_async(move |req: ResizeRequest| {
                let manager = manager.clone();
                async move { manager.resize(req).await.map_err(Error::Handler) }
            })
            .description("Resize a PTY session in terminal columns and rows.")
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::detach",
            RegisterFunction::new_async(move |req: DetachRequest| {
                let manager = manager.clone();
                async move { manager.detach(req).await.map_err(Error::Handler) }
            })
            .description("Detach a browser output target while retaining its PTY session.")
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::attach",
            RegisterFunction::new_async(move |req: AttachRequest| {
                let manager = manager.clone();
                async move { manager.attach(req).await.map_err(Error::Handler) }
            })
            .description("Attach to a retained PTY session and replay buffered output.")
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::adopt",
            RegisterFunction::new_async(move |req: AdoptRequest| {
                let manager = manager.clone();
                async move { manager.adopt(req).await.map_err(Error::Handler) }
            })
            .description(
                "Take back an unattached PTY session whose reconnect token is gone, from the console page that owns it. Refuses a session someone is attached to, and a page that is not the session's own.",
            )
            .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let manager = manager.clone();
        iii.register_function(
            "shell::pty::sessions",
            RegisterFunction::new_async(move |_req: SessionsRequest| {
                let manager = manager.clone();
                async move { Ok::<_, Error>(manager.sessions().await) }
            })
            .description(
                "Live PTY sessions with their program, cwd, sequence, replay buffer size, and output target. Diagnostics only — no credentials.",
            ),
        );
    }
    iii.register_function(
        "shell::pty::close",
        RegisterFunction::new_async(move |req: CloseRequest| {
            let manager = manager.clone();
            async move { manager.close(req).await.map_err(Error::Handler) }
        })
        .description("Terminate and close a PTY session.")
        .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
    );
}

fn spawn_process(
    cwd: &Path,
    cols: u16,
    rows: u16,
    program: Option<&str>,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> anyhow::Result<SpawnedPty> {
    let pair = native_pty_system().openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut command = match program {
        Some(program) => {
            let mut command = CommandBuilder::new(program);
            command.args(args);
            command
        }
        None => CommandBuilder::new_default_prog(),
    };
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "iii");
    // Per-call env last: a caller may override TERM* for its own program, and
    // the dangerous keys are already refused before the spawn.
    for (key, value) in env {
        command.env(key, value);
    }

    let child = pair.slave.spawn_command(command)?;
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    drop(pair.slave);

    Ok(SpawnedPty {
        master: pair.master,
        reader,
        writer,
        child,
    })
}

fn validate_size(cols: u16, rows: u16) -> Result<(), String> {
    if !(2..=500).contains(&cols) || !(2..=500).contains(&rows) {
        return Err("terminal size must be between 2 and 500 columns/rows".to_string());
    }
    Ok(())
}

/// Per-session env is deny-only, exactly like `shell::exec`'s per-call env:
/// the exec-hijacking keys are never settable, everything else is.
fn reject_dangerous_env(env: &BTreeMap<String, String>) -> Result<(), String> {
    for key in env.keys() {
        // Syntax first, and for the deny-list's sake: an environment entry is
        // one `key=value` string, so a key carrying its own `=` is checked
        // under one name and delivered under another — `PATH=/tmp/evil` passes
        // the `PATH` rule and still hands the child a `PATH`.
        if crate::exec::policy::is_invalid_env_key(key) {
            return Err(format!(
                "terminal env key '{key}' is not an environment variable name \
                 ([A-Za-z_][A-Za-z0-9_]*); remove it"
            ));
        }
        if crate::exec::policy::is_dangerous_env_key(key) {
            return Err(format!(
                "terminal env key '{key}' is never settable (exec-hijacking key); remove it"
            ));
        }
    }
    Ok(())
}

/// Output goes to a browser-registered console handler and nowhere else:
/// `iii::<worker>-ui::pty-output::console-<browser-id>`. The `<worker>-ui`
/// segment is not pinned to the shell's own page, because a worker that runs
/// its own program in a session (an agent CLI) owns its own console page and
/// therefore its own handler prefix. The SHAPE is what carries the guarantee
/// — a session cannot be pointed at an arbitrary function on the bus.
fn validate_output_function_id(function_id: &str) -> Result<(), String> {
    let malformed = || "terminal output function is not a console UI handler".to_string();
    let Some(rest) = function_id.strip_prefix("iii::") else {
        return Err(malformed());
    };
    let Some((ui, browser_id)) = rest.split_once(OUTPUT_FUNCTION_INFIX) else {
        return Err(malformed());
    };
    let ui_name = ui.strip_suffix("-ui").ok_or_else(malformed)?;
    if ui_name.is_empty()
        || ui_name.len() > 64
        || !ui_name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !ui_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(malformed());
    }
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

/// The `<name>` in `iii::<name>-ui::pty-output::console-<browser>`: which
/// console page a handler belongs to, with the browser it belongs to left out.
fn output_ui_name(function_id: &str) -> Option<&str> {
    function_id
        .strip_prefix("iii::")?
        .split_once(OUTPUT_FUNCTION_INFIX)?
        .0
        .strip_suffix("-ui")
}

fn require_caller(caller_worker_id: Option<&str>) -> Result<&str, String> {
    caller_worker_id
        .filter(|caller| !caller.is_empty())
        .ok_or_else(|| "terminal calls require engine-stamped caller identity".to_string())
}

fn validate_request_id(request_id: &str) -> Result<&str, String> {
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("terminal request id is invalid".to_string());
    }
    Ok(request_id)
}

async fn remove_session(
    sessions: &RwLock<HashMap<String, Arc<PtySession>>>,
    session_id: &str,
    expected: &Arc<PtySession>,
) {
    let mut sessions = sessions.write().await;
    let matches = sessions
        .get(session_id)
        .is_some_and(|session| Arc::ptr_eq(session, expected));
    if matches {
        sessions.remove(session_id);
    }
}

async fn terminate_session(session: Arc<PtySession>) -> Result<(), String> {
    #[cfg(unix)]
    {
        let process_groups = session_process_groups(&session)?;
        if !process_groups.is_empty() {
            let terminate_session = session.clone();
            return tokio::task::spawn_blocking(move || {
                terminate_process_groups(&terminate_session, process_groups)
            })
            .await
            .map_err(|error| format!("terminal close task failed: {error}"))?;
        }
    }
    if session_exited(&session)? {
        return Ok(());
    }
    let kill_session = session.clone();
    let kill_result = tokio::task::spawn_blocking(move || {
        let mut killer = kill_session
            .killer
            .lock()
            .map_err(|_| "terminal killer lock poisoned".to_string())?;
        killer
            .kill()
            .map_err(|error| format!("terminal close failed: {error}"))
    })
    .await
    .map_err(|error| format!("terminal close task failed: {error}"))?;
    if let Err(error) = kill_result {
        if session_exited(&session)? {
            return Ok(());
        }
        return Err(error);
    }
    wait_for_session_exit(&session).await
}

async fn wait_for_session_exit(session: &PtySession) -> Result<(), String> {
    tokio::time::timeout(PORTABLE_TERMINATION_WAIT, async {
        loop {
            if session_exited(session)? {
                return Ok(());
            }
            tokio::time::sleep(TERMINATION_POLL_INTERVAL).await;
        }
    })
    .await
    .map_err(|_| "terminal close timed out waiting for process exit".to_string())?
}

async fn reset_session_closing(session: &PtySession) {
    let mut lifecycle = session.lifecycle.lock().await;
    if !lifecycle.closed {
        lifecycle.closing = false;
    }
}

#[cfg(unix)]
fn session_process_groups(session: &PtySession) -> Result<Vec<libc::pid_t>, String> {
    let mut process_groups = Vec::new();
    if let Some(process_group) = *session
        .process_group
        .lock()
        .map_err(|_| "terminal process group lock poisoned".to_string())?
    {
        add_process_group(&mut process_groups, process_group)?;
    }
    let foreground = session
        .master
        .lock()
        .map_err(|_| "terminal master lock poisoned".to_string())?
        .process_group_leader();
    if let Some(process_group) = foreground {
        add_process_group(&mut process_groups, process_group)?;
    }
    Ok(process_groups)
}

#[cfg(unix)]
fn add_process_group(
    process_groups: &mut Vec<libc::pid_t>,
    process_group: libc::pid_t,
) -> Result<(), String> {
    if process_group <= 1 {
        return Err(format!(
            "terminal close refused invalid process group {process_group}"
        ));
    }
    let own_process_group = unsafe { libc::getpgrp() };
    if process_group == own_process_group {
        return Err("terminal close refused to signal the worker process group".to_string());
    }
    if !process_groups.contains(&process_group) {
        process_groups.push(process_group);
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_process_groups(
    session: &PtySession,
    mut process_groups: Vec<libc::pid_t>,
) -> Result<(), String> {
    let stages = [
        (libc::SIGHUP, SIGHUP_TERMINATION_WAIT),
        (libc::SIGTERM, SIGTERM_TERMINATION_WAIT),
        (libc::SIGKILL, SIGKILL_TERMINATION_WAIT),
    ];
    for (signal, wait) in stages {
        for process_group in session_process_groups(session)? {
            add_process_group(&mut process_groups, process_group)?;
        }
        for process_group in &process_groups {
            signal_process_group(*process_group, signal)?;
        }
        if wait_for_process_groups(&process_groups, wait)? {
            return Ok(());
        }
    }
    let survivors = process_groups
        .into_iter()
        .filter(|process_group| process_group_exists(*process_group).unwrap_or(true))
        .map(|process_group| process_group.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "terminal close timed out waiting for process group(s) {survivors}"
    ))
}

#[cfg(unix)]
fn signal_process_group(process_group: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
    if unsafe { libc::kill(-process_group, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(format!(
        "terminal close failed to signal process group {process_group}: {error}"
    ))
}

#[cfg(unix)]
fn process_group_exists(process_group: libc::pid_t) -> Result<bool, String> {
    if unsafe { libc::kill(-process_group, 0) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(format!(
            "terminal close failed to inspect process group {process_group}: {error}"
        )),
    }
}

#[cfg(unix)]
fn wait_for_process_groups(
    process_groups: &[libc::pid_t],
    timeout: StdDuration,
) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut any_running = false;
        for process_group in process_groups {
            any_running |= process_group_exists(*process_group)?;
        }
        if !any_running {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(TERMINATION_POLL_INTERVAL);
    }
}

#[cfg(unix)]
fn clear_terminated_process_group(session: &PtySession) {
    let Ok(mut process_group) = session.process_group.lock() else {
        return;
    };
    if process_group
        .is_some_and(|process_group| !process_group_exists(process_group).unwrap_or(false))
    {
        *process_group = None;
    }
}

async fn authenticate_session(
    session: &PtySession,
    access_key: &str,
    caller_worker_id: &str,
) -> Result<(), String> {
    session
        .control
        .lock()
        .await
        .authenticate(access_key, caller_worker_id)
}

async fn resize_session(session: Arc<PtySession>, cols: u16, rows: u16) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let master = session
            .master
            .lock()
            .map_err(|_| "terminal master lock poisoned".to_string())?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("terminal resize failed: {error}"))
    })
    .await
    .map_err(|error| format!("terminal resize task failed: {error}"))??;
    Ok(())
}

fn ensure_session_open(lifecycle: &SessionLifecycle) -> Result<(), String> {
    if lifecycle.closing || lifecycle.closed {
        Err("terminal session is closed".to_string())
    } else {
        Ok(())
    }
}

fn session_exited(session: &PtySession) -> Result<bool, String> {
    session
        .status
        .lock()
        .map(|status| matches!(*status, SessionStatus::Exited { .. }))
        .map_err(|_| "terminal status lock poisoned".to_string())
}

fn release_session_permit(session: &PtySession) -> Result<(), String> {
    session
        .permit
        .lock()
        .map_err(|_| "terminal permit lock poisoned".to_string())?
        .take();
    Ok(())
}

async fn emit_output(iii: &IIIClient, function_id: &str, event: OutputEvent) -> bool {
    let payload = match serde_json::to_value(event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(error = %error, "terminal output serialization failed");
            return false;
        }
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
            tracing::debug!(error = %error, "terminal output delivery failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Condvar;
    use std::time::Duration;

    use crate::code::config::CoderConfig;
    use crate::code::path::PathResolver;

    use super::*;

    #[derive(Debug)]
    struct HarnessOpen {
        session_id: String,
        access_key: String,
        reconnect_token: String,
    }

    struct HarnessAttach {
        access_key: String,
        reconnect_token: String,
        replay: Vec<OutputFrame>,
        truncated: bool,
        next_sequence: u64,
        status: SessionStatus,
    }

    #[derive(Debug)]
    struct FailingResizeMaster;

    impl MasterPty for FailingResizeMaster {
        fn resize(&self, _size: PtySize) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("forced resize failure"))
        }

        fn get_size(&self) -> anyhow::Result<PtySize> {
            Err(anyhow::anyhow!("test master has no size"))
        }

        fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
            Err(anyhow::anyhow!("test master has no reader"))
        }

        fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
            Err(anyhow::anyhow!("test master has no writer"))
        }

        #[cfg(unix)]
        fn process_group_leader(&self) -> Option<libc::pid_t> {
            None
        }

        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<portable_pty::unix::RawFd> {
            None
        }

        #[cfg(unix)]
        fn tty_name(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    #[derive(Debug)]
    struct FailingKiller;

    impl ChildKiller for FailingKiller {
        fn kill(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("forced kill failure"))
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(Self)
        }
    }

    struct BlockingWriter {
        entered: Option<std::sync::mpsc::Sender<()>>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
            }
            let (released, wake) = &*self.release;
            let mut released = released.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct PtyTestHarness {
        manager: PtyManager,
        caller_worker_id: String,
        cwd: String,
        output_function_id: String,
        _workspace: tempfile::TempDir,
    }

    impl PtyTestHarness {
        async fn new(program: &str) -> Self {
            let workspace = tempfile::tempdir().unwrap();
            let config = CoderConfig {
                base_paths: vec![workspace.path().to_path_buf()],
                ..CoderConfig::default()
            };
            let resolver = Arc::new(PathResolver::new(&config).unwrap());
            let manager = PtyManager::new(
                IIIClient::new("ws://127.0.0.1:1"),
                Arc::new(RwLock::new(resolver)),
            )
            .with_program(program);

            Self {
                manager,
                caller_worker_id: "pty-test-worker".to_string(),
                cwd: workspace.path().display().to_string(),
                output_function_id: format!("{OUTPUT_FUNCTION_PREFIX}{}", Uuid::new_v4()),
                _workspace: workspace,
            }
        }

        /// A manager with no configured program, so the request decides what
        /// the session runs — the shape another worker's console page uses.
        async fn unpinned() -> Self {
            let workspace = tempfile::tempdir().unwrap();
            let config = CoderConfig {
                base_paths: vec![workspace.path().to_path_buf()],
                ..CoderConfig::default()
            };
            let resolver = Arc::new(PathResolver::new(&config).unwrap());
            let manager = PtyManager::new(
                IIIClient::new("ws://127.0.0.1:1"),
                Arc::new(RwLock::new(resolver)),
            );

            Self {
                manager,
                caller_worker_id: "pty-test-worker".to_string(),
                cwd: workspace.path().display().to_string(),
                output_function_id: format!(
                    "iii::claude-ui::pty-output::console-{}",
                    Uuid::new_v4()
                ),
                _workspace: workspace,
            }
        }

        async fn open(&self) -> HarnessOpen {
            self.open_request(None, None, None).await.unwrap()
        }

        async fn open_request(
            &self,
            program: Option<&str>,
            args: Option<Vec<String>>,
            env: Option<BTreeMap<String, String>>,
        ) -> Result<HarnessOpen, String> {
            let opened = self
                .manager
                .open(OpenRequest {
                    program: program.map(str::to_string),
                    args,
                    env,
                    request_id: None,
                    cwd: self.cwd.clone(),
                    cols: 80,
                    rows: 24,
                    output_function_id: self.output_function_id.clone(),
                    caller_worker_id: Some(self.caller_worker_id.clone()),
                })
                .await?;

            Ok(HarnessOpen {
                session_id: opened.session_id,
                access_key: opened.access_key,
                reconnect_token: opened.reconnect_token,
            })
        }

        async fn write(&self, opened: &HarnessOpen, data: &[u8]) {
            let session = self.manager.session(&opened.session_id).await.unwrap();
            let next_sequence = session.output.lock().unwrap().frames_after(0).next_sequence;
            self.manager
                .write(WriteRequest {
                    session_id: opened.session_id.clone(),
                    access_key: opened.access_key.clone(),
                    data: BASE64_STANDARD.encode(data),
                    caller_worker_id: Some(self.caller_worker_id.clone()),
                })
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let current = session.output.lock().unwrap().frames_after(0).next_sequence;
                    if current > next_sequence {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("PTY output was buffered");
        }

        async fn wait_for_output(&self, session_id: &str, marker: &str) {
            let session = self.manager.session(session_id).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let found = session
                        .output
                        .lock()
                        .unwrap()
                        .frames_after(0)
                        .frames
                        .iter()
                        .any(|frame| decode_frame(frame).contains(marker));
                    if found {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("expected PTY output was buffered");
        }

        async fn detach(&self, opened: &HarnessOpen) {
            self.manager
                .detach(DetachRequest {
                    session_id: opened.session_id.clone(),
                    access_key: opened.access_key.clone(),
                    caller_worker_id: Some(self.caller_worker_id.clone()),
                })
                .await
                .unwrap();
        }

        async fn attach(
            &self,
            session_id: &str,
            reconnect_token: &str,
            after_sequence: u64,
        ) -> HarnessAttach {
            let attached = self
                .manager
                .attach(AttachRequest {
                    request_id: None,
                    session_id: session_id.to_string(),
                    reconnect_token: reconnect_token.to_string(),
                    output_function_id: self.output_function_id.clone(),
                    cols: 80,
                    rows: 24,
                    after_sequence,
                    caller_worker_id: Some(self.caller_worker_id.clone()),
                })
                .await
                .unwrap();

            HarnessAttach {
                access_key: attached.access_key,
                reconnect_token: attached.reconnect_token,
                replay: attached.frames,
                truncated: attached.truncated,
                next_sequence: attached.next_sequence,
                status: attached.status,
            }
        }

        async fn close(&self, session_id: &str, access_key: &str) -> CloseResponse {
            self.manager
                .close(CloseRequest {
                    session_id: session_id.to_string(),
                    access_key: access_key.to_string(),
                    caller_worker_id: Some(self.caller_worker_id.clone()),
                })
                .await
                .unwrap()
        }

        async fn pid(&self, session_id: &str) -> Option<u32> {
            self.manager.session(session_id).await.unwrap().pid
        }

        async fn close_all(&self) {
            self.manager.close_all().await.unwrap();
            assert_eq!(self.manager.permits.available_permits(), MAX_SESSIONS);
        }
    }

    fn decode_frame(frame: &OutputFrame) -> String {
        String::from_utf8_lossy(&BASE64_STANDARD.decode(&frame.data).unwrap()).into_owned()
    }

    #[tokio::test]
    async fn open_returns_usable_session_reconnect_token() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness
            .manager
            .open(OpenRequest {
                program: None,
                args: None,
                env: None,
                request_id: None,
                cwd: harness.cwd.clone(),
                cols: 80,
                rows: 24,
                output_function_id: harness.output_function_id.clone(),
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await
            .unwrap();
        let session = harness.manager.session(&opened.session_id).await.unwrap();

        assert_eq!(
            opened.reconnect_token,
            session.control.lock().await.reconnect_token()
        );
        harness
            .manager
            .detach(DetachRequest {
                session_id: opened.session_id.clone(),
                access_key: opened.access_key,
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await
            .unwrap();
        let attached = harness
            .attach(&opened.session_id, &opened.reconnect_token, 0)
            .await;
        assert!(
            harness
                .close(&opened.session_id, &attached.access_key)
                .await
                .closed
        );
    }

    #[tokio::test]
    async fn retried_open_request_returns_the_same_session() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let request_id = Uuid::new_v4().to_string();
        let open = || OpenRequest {
            program: None,
            args: None,
            env: None,
            request_id: Some(request_id.clone()),
            cwd: harness.cwd.clone(),
            cols: 80,
            rows: 24,
            output_function_id: harness.output_function_id.clone(),
            caller_worker_id: Some(harness.caller_worker_id.clone()),
        };

        let first = harness.manager.open(open()).await.unwrap();
        let retried = harness.manager.open(open()).await.unwrap();

        assert_eq!(first.session_id, retried.session_id);
        assert_eq!(first.access_key, retried.access_key);
        assert_eq!(harness.manager.sessions.read().await.len(), 1);
        harness.close(&first.session_id, &first.access_key).await;
    }

    #[tokio::test]
    async fn retried_attach_request_returns_the_same_rotated_credentials() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;
        let request_id = Uuid::new_v4().to_string();
        let attach = || AttachRequest {
            request_id: Some(request_id.clone()),
            session_id: opened.session_id.clone(),
            reconnect_token: opened.reconnect_token.clone(),
            output_function_id: harness.output_function_id.clone(),
            cols: 80,
            rows: 24,
            after_sequence: 0,
            caller_worker_id: Some(harness.caller_worker_id.clone()),
        };

        let first = harness.manager.attach(attach()).await.unwrap();
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        assert!(session
            .control
            .lock()
            .await
            .detach_output_target(&harness.output_function_id, &first.access_key));
        let retry_worker = "retry-worker";
        let retry_output = format!("{OUTPUT_FUNCTION_PREFIX}{}", Uuid::new_v4());
        let retried = harness
            .manager
            .attach(AttachRequest {
                request_id: Some(request_id),
                session_id: opened.session_id.clone(),
                reconnect_token: opened.reconnect_token,
                output_function_id: retry_output.clone(),
                cols: 80,
                rows: 24,
                after_sequence: 0,
                caller_worker_id: Some(retry_worker.to_string()),
            })
            .await
            .unwrap();

        assert_eq!(first.access_key, retried.access_key);
        assert_eq!(first.reconnect_token, retried.reconnect_token);
        let control = session.control.lock().await;
        assert_eq!(control.status(), SessionStatus::Attached);
        assert_eq!(control.output_function_id(), retry_output);
        assert!(control
            .authenticate(&retried.access_key, retry_worker)
            .is_ok());
        drop(control);
        harness
            .manager
            .close(CloseRequest {
                session_id: opened.session_id,
                access_key: first.access_key,
                caller_worker_id: Some(retry_worker.to_string()),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn queued_write_is_rejected_after_close_wins_session_lock() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        let session_lock = session.lifecycle.lock().await;
        let close_manager = harness.manager.clone();
        let close_session_id = opened.session_id.clone();
        let close_access_key = opened.access_key.clone();
        let caller_worker_id = harness.caller_worker_id.clone();
        let close_task = tokio::spawn(async move {
            close_manager
                .close(CloseRequest {
                    session_id: close_session_id,
                    access_key: close_access_key,
                    caller_worker_id: Some(caller_worker_id),
                })
                .await
        });
        tokio::task::yield_now().await;

        let write_manager = harness.manager.clone();
        let write_session_id = opened.session_id;
        let write_access_key = opened.access_key;
        let caller_worker_id = harness.caller_worker_id.clone();
        let write_task = tokio::spawn(async move {
            write_manager
                .write(WriteRequest {
                    session_id: write_session_id,
                    access_key: write_access_key,
                    data: BASE64_STANDARD.encode(b"echo should-not-run\r"),
                    caller_worker_id: Some(caller_worker_id),
                })
                .await
        });
        tokio::task::yield_now().await;
        drop(session_lock);

        assert!(close_task.await.unwrap().unwrap().closed);
        assert_eq!(
            write_task.await.unwrap().unwrap_err(),
            "terminal session is closed"
        );
    }

    #[tokio::test]
    async fn blocked_write_does_not_prevent_close_and_close_wins() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let original_writer = {
            let mut writer = session.writer.lock().unwrap();
            std::mem::replace(
                &mut *writer,
                Box::new(BlockingWriter {
                    entered: Some(entered_tx),
                    release: release.clone(),
                }),
            )
        };
        let write_manager = harness.manager.clone();
        let write_session_id = opened.session_id.clone();
        let write_access_key = opened.access_key.clone();
        let caller_worker_id = harness.caller_worker_id.clone();
        let write_task = tokio::spawn(async move {
            write_manager
                .write(WriteRequest {
                    session_id: write_session_id,
                    access_key: write_access_key,
                    data: BASE64_STANDARD.encode(b"blocked input"),
                    caller_worker_id: Some(caller_worker_id),
                })
                .await
        });
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("writer entered blocking write")
        })
        .await
        .unwrap();

        let close = tokio::time::timeout(
            Duration::from_secs(3),
            harness.close(&opened.session_id, &opened.access_key),
        )
        .await
        .expect("close completed while the writer remained blocked");
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();

        assert!(close.closed);
        assert_eq!(
            write_task.await.unwrap().unwrap_err(),
            "terminal session is closed"
        );
        drop(original_writer);
    }

    #[tokio::test]
    async fn queued_attach_is_rejected_after_close_wins_session_lock() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        let session_lock = session.lifecycle.lock().await;
        let close_manager = harness.manager.clone();
        let close_session_id = opened.session_id.clone();
        let close_access_key = opened.access_key;
        let caller_worker_id = harness.caller_worker_id.clone();
        let close_task = tokio::spawn(async move {
            close_manager
                .close(CloseRequest {
                    session_id: close_session_id,
                    access_key: close_access_key,
                    caller_worker_id: Some(caller_worker_id),
                })
                .await
        });
        tokio::task::yield_now().await;

        let attach_manager = harness.manager.clone();
        let attach_session_id = opened.session_id;
        let reconnect_token = opened.reconnect_token;
        let output_function_id = harness.output_function_id.clone();
        let caller_worker_id = harness.caller_worker_id.clone();
        let attach_task = tokio::spawn(async move {
            attach_manager
                .attach(AttachRequest {
                    request_id: None,
                    session_id: attach_session_id,
                    reconnect_token,
                    output_function_id,
                    cols: 80,
                    rows: 24,
                    after_sequence: 0,
                    caller_worker_id: Some(caller_worker_id),
                })
                .await
        });
        tokio::task::yield_now().await;
        drop(session_lock);

        assert!(close_task.await.unwrap().unwrap().closed);
        assert_eq!(
            attach_task.await.unwrap().unwrap_err(),
            "terminal session is closed"
        );
    }

    #[tokio::test]
    async fn close_all_waits_for_open_before_insertion() {
        let mut harness = PtyTestHarness::new("/bin/sh").await;
        let pause = Arc::new(OpenPause {
            reached: Notify::new(),
            release: Notify::new(),
        });
        harness.manager.open_pause = Some(pause.clone());
        let open_manager = harness.manager.clone();
        let cwd = harness.cwd.clone();
        let output_function_id = harness.output_function_id.clone();
        let caller_worker_id = harness.caller_worker_id.clone();
        let open_task = tokio::spawn(async move {
            open_manager
                .open(OpenRequest {
                    program: None,
                    args: None,
                    env: None,
                    request_id: None,
                    cwd,
                    cols: 80,
                    rows: 24,
                    output_function_id,
                    caller_worker_id: Some(caller_worker_id),
                })
                .await
        });
        pause.reached.notified().await;

        let close_manager = harness.manager.clone();
        let close_task = tokio::spawn(async move { close_manager.close_all().await });
        tokio::task::yield_now().await;
        let close_finished_before_release = close_task.is_finished();
        pause.release.notify_one();
        let opened = open_task.await.unwrap().unwrap();
        let closed = close_task.await.unwrap().unwrap();
        let survived_shutdown = harness.manager.session(&opened.session_id).await.is_ok();
        if survived_shutdown {
            harness.close(&opened.session_id, &opened.access_key).await;
        }

        assert!(!close_finished_before_release);
        assert_eq!(closed, 1);
        assert!(!survived_shutdown);
    }

    #[tokio::test]
    async fn close_all_reports_kill_failure_without_losing_session() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        #[cfg(unix)]
        let original_process_group = session.process_group.lock().unwrap().take();
        #[cfg(unix)]
        let original_master = {
            let mut master = session.master.lock().unwrap();
            std::mem::replace(&mut *master, Box::new(FailingResizeMaster))
        };
        let original_killer = {
            let mut killer = session.killer.lock().unwrap();
            std::mem::replace(&mut *killer, Box::new(FailingKiller))
        };

        let error = harness.manager.close_all().await.unwrap_err();
        let survived_failure = harness.manager.session(&opened.session_id).await.is_ok();
        let available_permits = harness.manager.permits.available_permits();
        {
            let mut killer = session.killer.lock().unwrap();
            *killer = original_killer;
        }
        #[cfg(unix)]
        {
            *session.process_group.lock().unwrap() = original_process_group;
            *session.master.lock().unwrap() = original_master;
        }
        let retried = harness.manager.close_all().await.unwrap();

        assert!(error.contains("forced kill failure"));
        assert!(survived_failure);
        assert_eq!(available_permits, MAX_SESSIONS - 1);
        assert_eq!(retried, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn close_escalates_until_hup_ignoring_descendant_exits() {
        use std::os::unix::fs::PermissionsExt;

        let mut harness = PtyTestHarness::new("/bin/sh").await;
        let script = harness._workspace.path().join("ignore-hup.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ntrap '' HUP TERM\nsh -c 'trap \"\" HUP TERM; while :; do sleep 1; done' &\nchild=$!\nprintf '__HUP_CHILD__:%s\\n' \"$child\"\nwhile :; do sleep 1; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        harness.manager.program = Some(Arc::from(script.to_string_lossy().into_owned()));
        let opened = harness.open().await;
        harness
            .wait_for_output(&opened.session_id, "__HUP_CHILD__:")
            .await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        let output = session
            .output
            .lock()
            .unwrap()
            .frames_after(0)
            .frames
            .iter()
            .map(decode_frame)
            .collect::<String>();
        let child_pid = output
            .split("__HUP_CHILD__:")
            .nth(1)
            .and_then(|suffix| {
                let digits = suffix
                    .chars()
                    .take_while(|character| character.is_ascii_digit())
                    .collect::<String>();
                digits.parse::<libc::pid_t>().ok()
            })
            .expect("descendant pid was printed");
        let process_group = session
            .process_group
            .lock()
            .unwrap()
            .expect("PTY process group");
        signal_process_group(process_group, libc::SIGHUP).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(process_exists(child_pid));

        assert!(
            tokio::time::timeout(
                Duration::from_secs(3),
                harness.close(&opened.session_id, &opened.access_key),
            )
            .await
            .expect("close completed after escalation")
            .closed
        );
        assert!(!process_exists(child_pid));
        assert_eq!(harness.manager.permits.available_permits(), MAX_SESSIONS);
    }

    #[cfg(unix)]
    fn process_exists(pid: libc::pid_t) -> bool {
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    #[tokio::test]
    async fn detached_session_replays_output() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let pid = harness.pid(&opened.session_id).await;

        harness.detach(&opened).await;
        harness
            .write(&opened, b"printf '__DETACHED_%s__\\n' OK\r")
            .await;
        harness
            .wait_for_output(&opened.session_id, "__DETACHED_OK__")
            .await;
        let attached = harness
            .attach(&opened.session_id, &opened.reconnect_token, 0)
            .await;

        assert_eq!(harness.pid(&opened.session_id).await, pid);
        assert!(attached
            .replay
            .iter()
            .any(|frame| decode_frame(frame).contains("__DETACHED_OK__")));
        assert!(attached
            .replay
            .windows(2)
            .all(|frames| frames[0].sequence < frames[1].sequence));
        assert!(!attached.truncated);
        assert_eq!(
            attached.next_sequence,
            attached
                .replay
                .last()
                .map_or(1, |frame| frame.sequence.saturating_add(1))
        );
        harness.close_all().await;
    }

    #[tokio::test]
    async fn invalid_access_key_cannot_detach_session() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;

        let result = harness
            .manager
            .detach(DetachRequest {
                session_id: opened.session_id,
                access_key: "invalid".to_string(),
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await;

        assert_eq!(
            result.unwrap_err(),
            "terminal session credentials are invalid"
        );
        harness.close_all().await;
    }

    #[tokio::test]
    async fn matching_delivery_failure_detaches_without_closing_session() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let pid = harness.pid(&opened.session_id).await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        let (output_function_id, access_key) = output_target(&session)
            .await
            .expect("attached output target");

        detach_failed_output(&session, &output_function_id, &access_key).await;

        assert_eq!(*session.status.lock().unwrap(), SessionStatus::Detached);
        assert_eq!(harness.pid(&opened.session_id).await, pid);
        assert!(
            harness
                .close(&opened.session_id, &opened.access_key)
                .await
                .closed
        );
    }

    #[tokio::test]
    async fn reused_reconnect_token_is_rejected() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;

        let attached = harness
            .attach(&opened.session_id, &opened.reconnect_token, 0)
            .await;
        let reused = harness
            .manager
            .attach(AttachRequest {
                request_id: None,
                session_id: opened.session_id.clone(),
                reconnect_token: opened.reconnect_token.clone(),
                output_function_id: harness.output_function_id.clone(),
                cols: 80,
                rows: 24,
                after_sequence: 0,
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await;

        assert_eq!(reused.unwrap_err(), "terminal reconnect token is invalid");
        assert_ne!(attached.access_key, opened.access_key);
        assert_ne!(attached.reconnect_token, opened.reconnect_token);
        assert_eq!(attached.status, SessionStatus::Attached);
        assert!(
            harness
                .close(&opened.session_id, &attached.access_key)
                .await
                .closed
        );
    }

    #[tokio::test]
    async fn failed_resize_preserves_valid_reconnect_token() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();
        let original_master = {
            let mut master = session.master.lock().unwrap();
            std::mem::replace(&mut *master, Box::new(FailingResizeMaster))
        };

        let failed = harness
            .manager
            .attach(AttachRequest {
                request_id: None,
                session_id: opened.session_id.clone(),
                reconnect_token: opened.reconnect_token.clone(),
                output_function_id: harness.output_function_id.clone(),
                cols: 80,
                rows: 24,
                after_sequence: 0,
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await;
        assert!(failed.unwrap_err().contains("forced resize failure"));

        {
            let mut master = session.master.lock().unwrap();
            *master = original_master;
        }
        let attached = harness
            .attach(&opened.session_id, &opened.reconnect_token, 0)
            .await;
        assert!(
            harness
                .close(&opened.session_id, &attached.access_key)
                .await
                .closed
        );
    }

    #[tokio::test]
    async fn concurrent_attach_consumes_reconnect_token_once() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;
        let request = || AttachRequest {
            request_id: None,
            session_id: opened.session_id.clone(),
            reconnect_token: opened.reconnect_token.clone(),
            output_function_id: harness.output_function_id.clone(),
            cols: 80,
            rows: 24,
            after_sequence: 0,
            caller_worker_id: Some(harness.caller_worker_id.clone()),
        };

        let (first, second) = tokio::join!(
            harness.manager.attach(request()),
            harness.manager.attach(request())
        );
        let (attached, rejected) = match (first, second) {
            (Ok(attached), Err(rejected)) | (Err(rejected), Ok(attached)) => (attached, rejected),
            results => panic!("expected one successful attach, got {results:?}"),
        };

        assert_eq!(rejected, "terminal reconnect token is invalid");
        assert!(
            harness
                .close(&opened.session_id, &attached.access_key)
                .await
                .closed
        );
    }

    #[tokio::test]
    async fn a_page_adopts_its_own_orphan_without_a_token() {
        // What a browser that lost its storage looks like from here: the
        // session is detached, and the caller has no reconnect token to offer.
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;
        let another_browser = format!("{OUTPUT_FUNCTION_PREFIX}{}", Uuid::new_v4());

        let adopted = harness
            .manager
            .adopt(AdoptRequest {
                session_id: opened.session_id.clone(),
                output_function_id: another_browser.clone(),
                cols: 80,
                rows: 24,
                after_sequence: 0,
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await
            .unwrap();

        assert_ne!(adopted.access_key, opened.access_key);
        assert_ne!(adopted.reconnect_token, opened.reconnect_token);
        // The old credentials are dead, so the previous owner cannot write to
        // a terminal it no longer holds.
        assert!(harness
            .manager
            .attach(AttachRequest {
                request_id: None,
                session_id: opened.session_id.clone(),
                reconnect_token: opened.reconnect_token.clone(),
                output_function_id: another_browser,
                cols: 80,
                rows: 24,
                after_sequence: 0,
                caller_worker_id: Some(harness.caller_worker_id.clone()),
            })
            .await
            .is_err());
        assert!(
            harness
                .close(&opened.session_id, &adopted.access_key)
                .await
                .closed
        );
    }

    #[tokio::test]
    async fn adoption_refuses_a_live_terminal_and_a_foreign_page() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        let request = |output: String| AdoptRequest {
            session_id: opened.session_id.clone(),
            output_function_id: output,
            cols: 80,
            rows: 24,
            after_sequence: 0,
            caller_worker_id: Some(harness.caller_worker_id.clone()),
        };

        // Someone is watching this terminal: it is not up for adoption.
        let attached = harness
            .manager
            .adopt(request(format!(
                "{OUTPUT_FUNCTION_PREFIX}{}",
                Uuid::new_v4()
            )))
            .await;
        assert_eq!(
            attached.unwrap_err(),
            "terminal session is attached; detach it before adopting"
        );

        // Detached, but another worker's page may not claim it.
        harness.detach(&opened).await;
        let foreign = harness
            .manager
            .adopt(request(format!(
                "iii::pi-cli-ui::pty-output::console-{}",
                Uuid::new_v4()
            )))
            .await;
        assert_eq!(
            foreign.unwrap_err(),
            "terminal session belongs to another console page"
        );

        harness.close_all().await;
    }

    #[tokio::test]
    async fn explicit_close_releases_detached_session_permit() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;

        assert_eq!(
            harness.manager.permits.available_permits(),
            MAX_SESSIONS - 1
        );
        assert!(
            harness
                .close(&opened.session_id, &opened.access_key)
                .await
                .closed
        );
        assert_eq!(harness.manager.permits.available_permits(), MAX_SESSIONS);
    }

    #[tokio::test]
    async fn close_all_releases_all_session_permits() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        for _ in 0..MAX_SESSIONS {
            harness.open().await;
        }

        assert_eq!(harness.manager.permits.available_permits(), 0);
        harness.close_all().await;
    }

    #[tokio::test]
    async fn exited_session_is_retained_until_explicit_close() {
        let harness = PtyTestHarness::new("/bin/sh").await;
        let opened = harness.open().await;
        harness.detach(&opened).await;
        harness.write(&opened, b"exit\r").await;
        let session = harness.manager.session(&opened.session_id).await.unwrap();

        let status = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let status = session.status.lock().unwrap().clone();
                if matches!(status, SessionStatus::Exited { .. }) {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("PTY exit status was retained");

        assert!(matches!(
            status,
            SessionStatus::Exited {
                exit_code: Some(0),
                ..
            }
        ));
        assert_eq!(
            harness.manager.permits.available_permits(),
            MAX_SESSIONS - 1
        );
        let attached = harness
            .attach(&opened.session_id, &opened.reconnect_token, 0)
            .await;
        assert!(matches!(
            attached.status,
            SessionStatus::Exited {
                exit_code: Some(0),
                ..
            }
        ));
        assert!(
            harness
                .close(&opened.session_id, &attached.access_key)
                .await
                .closed
        );
        assert_eq!(harness.manager.permits.available_permits(), MAX_SESSIONS);
    }

    #[test]
    fn interactive_shell_preserves_cwd_and_round_trips_input() {
        let tmp = tempfile::tempdir().unwrap();
        let expected = tmp.path().canonicalize().unwrap();
        let mut pty =
            spawn_process(&expected, 80, 24, Some("/bin/sh"), &[], &BTreeMap::new()).unwrap();
        pty.master
            .resize(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let size = pty.master.get_size().unwrap();
        assert_eq!((size.cols, size.rows), (120, 40));

        let mut reader = pty.reader;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut chunk) {
                if read == 0 || tx.send(chunk[..read].to_vec()).is_err() {
                    break;
                }
            }
        });

        write!(pty.writer, "printf '__PTY_OK__:%s\\n' \"$PWD\"\r").unwrap();
        pty.writer.flush().unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut output = Vec::new();
        let marker = format!("__PTY_OK__:{}", expected.display());
        while std::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let chunk = rx.recv_timeout(remaining).expect("PTY produced output");
            output.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&output).contains(&marker) {
                break;
            }
        }
        let output = String::from_utf8_lossy(&output);
        assert!(
            output.contains(&marker),
            "interactive shell output: {output:?}"
        );

        write!(pty.writer, "exit\r").unwrap();
        pty.writer.flush().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if pty.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        pty.child.kill().unwrap();
    }

    #[test]
    fn output_target_is_scoped_to_a_console_browser() {
        let browser = Uuid::new_v4();
        assert!(validate_output_function_id(&format!("{OUTPUT_FUNCTION_PREFIX}{browser}")).is_ok());
        assert!(validate_output_function_id("harness::run").is_err());
        assert!(
            validate_output_function_id(&format!("{OUTPUT_FUNCTION_PREFIX}mep4u3o0-a1b2c3d4"))
                .is_ok()
        );
        assert!(validate_output_function_id(&format!("{OUTPUT_FUNCTION_PREFIX}bad_id")).is_err());
    }

    #[test]
    fn output_target_accepts_another_workers_console_page() {
        let browser = Uuid::new_v4();
        // A worker that runs its own program in a session serves its own
        // console page, so its handler prefix is its own.
        assert!(validate_output_function_id(&format!(
            "iii::claude-ui::pty-output::console-{browser}"
        ))
        .is_ok());
        assert!(validate_output_function_id(&format!(
            "iii::pi-cli-ui::pty-output::console-{browser}"
        ))
        .is_ok());
        // Still a console UI handler and nothing else.
        assert!(validate_output_function_id(&format!(
            "iii::claude::pty-output::console-{browser}"
        ))
        .is_err());
        assert!(validate_output_function_id(&format!(
            "worker::claude-ui::pty-output::console-{browser}"
        ))
        .is_err());
        assert!(validate_output_function_id(&format!(
            "iii::Claude_Code-ui::pty-output::console-{browser}"
        ))
        .is_err());
        assert!(validate_output_function_id("iii::-ui::pty-output::console-abc").is_err());
        assert!(
            validate_output_function_id("iii::shell-ui::pty-output::console-").is_err(),
            "an empty browser id is not a target"
        );
    }

    #[tokio::test]
    async fn a_requested_program_runs_instead_of_the_login_shell() {
        let harness = PtyTestHarness::unpinned().await;
        let opened = harness
            .open_request(
                Some("/bin/sh"),
                Some(vec![
                    "-c".to_string(),
                    "echo iii-program-marker; sleep 30".to_string(),
                ]),
                None,
            )
            .await
            .expect("session opens with a program");

        harness
            .wait_for_output(&opened.session_id, "iii-program-marker")
            .await;

        let sessions = harness.manager.sessions().await.sessions;
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.program.as_deref(), Some("/bin/sh"));
        assert!(session.sequence > 0, "{session:?}");
        assert!(session.frames > 0, "{session:?}");
        assert!(session.frame_bytes > 0, "{session:?}");
        assert!(!session.truncated, "{session:?}");
        assert_eq!(
            session.output_function_id.as_deref(),
            Some(harness.output_function_id.as_str())
        );

        harness.detach(&opened).await;
        let detached = &harness.manager.sessions().await.sessions[0];
        assert!(
            detached.output_function_id.is_none(),
            "a detached session has no output target: {detached:?}"
        );

        let _ = harness.manager.close_all().await;
    }

    #[tokio::test]
    async fn requested_env_reaches_the_program() {
        let harness = PtyTestHarness::unpinned().await;
        let mut env = BTreeMap::new();
        env.insert("III_ACTIVITY_URL".to_string(), "iii-env-marker".to_string());
        let opened = harness
            .open_request(
                Some("/bin/sh"),
                Some(vec![
                    "-c".to_string(),
                    "printf '%s\\n' \"$III_ACTIVITY_URL\"; sleep 30".to_string(),
                ]),
                Some(env),
            )
            .await
            .expect("session opens with env");

        harness
            .wait_for_output(&opened.session_id, "iii-env-marker")
            .await;
        let _ = harness.manager.close_all().await;
    }

    #[tokio::test]
    async fn a_dangerous_env_key_refuses_the_session() {
        let harness = PtyTestHarness::unpinned().await;
        let mut env = BTreeMap::new();
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        let error = harness
            .open_request(Some("/bin/sh"), None, Some(env))
            .await
            .expect_err("an exec-hijacking key must refuse the session");
        assert!(error.contains("LD_PRELOAD"), "{error}");
        assert!(
            harness.manager.sessions().await.sessions.is_empty(),
            "a refused open leaves no session behind"
        );
    }

    #[test]
    fn session_env_refuses_exec_hijacking_keys() {
        let mut env = BTreeMap::new();
        env.insert("III_AGENT".to_string(), "claude".to_string());
        assert!(reject_dangerous_env(&env).is_ok());

        for key in ["PATH", "LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "BASH_ENV"] {
            let mut env = BTreeMap::new();
            env.insert(key.to_string(), "/tmp/evil".to_string());
            let error = reject_dangerous_env(&env).expect_err("dangerous key must be refused");
            assert!(error.contains(key), "{error}");
        }
    }

    #[test]
    fn terminal_dimensions_are_bounded() {
        assert!(validate_size(80, 24).is_ok());
        assert!(validate_size(1, 24).is_err());
        assert!(validate_size(80, 501).is_err());
    }

    #[test]
    fn request_accepts_engine_stamped_caller_identity() {
        let request: CloseRequest = serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "access_key": "key",
            "_caller_worker_id": "worker-1"
        }))
        .unwrap();
        assert_eq!(request.caller_worker_id.as_deref(), Some("worker-1"));
    }
}
