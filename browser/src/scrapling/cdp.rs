//! Private raw Chrome DevTools Protocol connection foundation.
//!
//! Chromium's `--remote-debugging-pipe` protocol is UTF-8 JSON terminated by
//! a NUL byte. The browser reads commands from descriptor 3 and writes events
//! and responses to descriptor 4. Launch code owns creating/inheriting those
//! descriptors; this module owns framing, routing, cancellation and teardown.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io::{Read, Write};
use std::pin::Pin;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll};
use std::thread::{self, JoinHandle};

use futures::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

pub const REMOTE_DEBUGGING_PIPE_ARG: &str = "--remote-debugging-pipe";
pub const MAX_CDP_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum CdpError {
    Closed,
    Transport(String),
    InvalidMessage(String),
    Protocol {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    UnsupportedTransport(String),
}

impl fmt::Display for CdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("CDP connection is closed"),
            Self::Transport(message) | Self::InvalidMessage(message) => f.write_str(message),
            Self::Protocol { code, message, .. } => write!(f, "CDP error {code}: {message}"),
            Self::UnsupportedTransport(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CdpError {}

#[derive(Clone, Debug, PartialEq)]
pub struct CdpEvent {
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

#[derive(Debug)]
pub enum EventError {
    Closed,
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("CDP event stream is closed"),
        }
    }
}

impl std::error::Error for EventError {}

pub struct EventReceiver {
    receiver: broadcast::Receiver<CdpEvent>,
    disconnected: watch::Receiver<bool>,
    session_id: Option<String>,
    /// Deliver events from EVERY session, not just `session_id`. Needed by
    /// the child-target router: auto-attached OOPIF/worker targets announce
    /// themselves tagged with their PARENT page's sessionId, which a
    /// root-scoped receiver would filter out.
    any_session: bool,
}

impl EventReceiver {
    pub async fn recv(&mut self) -> Result<CdpEvent, EventError> {
        loop {
            if *self.disconnected.borrow() {
                return Err(EventError::Closed);
            }
            tokio::select! {
                changed = self.disconnected.changed() => {
                    if changed.is_err() || *self.disconnected.borrow() {
                        return Err(EventError::Closed);
                    }
                }
                event = self.receiver.recv() => match event {
                    Ok(event) if self.any_session || event.session_id == self.session_id => {
                        return Ok(event);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Closed) => return Err(EventError::Closed),
                    // Lagged is recoverable: the receiver skipped old events
                    // and can keep going. Surfacing it as an error made every
                    // consumer treat a busy page as a dead connection (the
                    // child-target router died forever; waits hard-failed
                    // pages that loaded fine). A waiter that missed its event
                    // falls back to its own deadline instead.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        }
    }
}

struct Pending {
    session_id: Option<String>,
    sender: oneshot::Sender<Result<Value, CdpError>>,
}

enum WriterCommand {
    Frame(Vec<u8>),
    Shutdown,
}

#[derive(Default)]
struct ProcessState {
    child: Option<Child>,
    status: Option<ExitStatus>,
}

struct Inner {
    next_id: AtomicU64,
    closed: AtomicBool,
    pending: Mutex<HashMap<u64, Pending>>,
    events: broadcast::Sender<CdpEvent>,
    disconnected: watch::Sender<bool>,
    writer_tx: mpsc::UnboundedSender<WriterCommand>,
    writer_thread: Mutex<Option<JoinHandle<()>>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    websocket_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    process: Mutex<ProcessState>,
}

impl Inner {
    fn terminate(&self, reason: CdpError) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let callbacks = std::mem::take(&mut *lock(&self.pending));
        for (_, pending) in callbacks {
            let _ = pending.sender.send(Err(reason.clone()));
        }
        let _ = self.writer_tx.send(WriterCommand::Shutdown);
        let _ = self.disconnected.send(true);
        self.terminate_process();
    }

    fn terminate_process(&self) {
        let mut process = lock(&self.process);
        let Some(mut child) = process.child.take() else {
            return;
        };
        let status = match child.try_wait() {
            Ok(Some(status)) => Some(status),
            Ok(None) => child.kill().and_then(|()| child.wait()).ok(),
            Err(_) => child.kill().and_then(|()| child.wait()).ok(),
        };
        process.status = status;
    }

    fn route(&self, message: Value) -> Result<(), CdpError> {
        let object = message
            .as_object()
            .ok_or_else(|| CdpError::InvalidMessage("CDP message is not an object".to_string()))?;
        let session_id = object
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);

        if object.get("id").and_then(Value::as_i64) == Some(-9999) {
            return Ok(());
        }
        if object.contains_key("id") {
            let id = object.get("id").and_then(Value::as_u64).ok_or_else(|| {
                CdpError::InvalidMessage("CDP response has an invalid id".to_string())
            })?;
            let mut pending = lock(&self.pending);
            // Command ids are client-global, so the id alone identifies the
            // command. Chrome emits some id-bearing ERROR frames untagged
            // (e.g. "Session with given id not found") — dropping those on a
            // sessionId mismatch orphans the await forever. Only a frame
            // tagged with a DIFFERENT session is rejected.
            let matches_session = pending
                .get(&id)
                .is_some_and(|callback| session_id.is_none() || callback.session_id == session_id);
            if !matches_session {
                return Ok(());
            }
            let callback = pending.remove(&id).expect("checked pending command");
            let result = if let Some(error) = object.get("error").and_then(Value::as_object) {
                Err(CdpError::Protocol {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown protocol error")
                        .to_string(),
                    data: error.get("data").cloned(),
                })
            } else {
                Ok(object.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = callback.sender.send(result);
            return Ok(());
        }

        let method = object
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| CdpError::InvalidMessage("CDP event has no method".to_string()))?;
        let _ = self.events.send(CdpEvent {
            session_id,
            method: method.to_string(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        });
        Ok(())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.writer_tx.send(WriterCommand::Shutdown);
        let _ = self.disconnected.send(true);
        if let Some(task) = lock(&self.websocket_task).take() {
            task.abort();
        }
        self.terminate_process();
    }
}

#[derive(Clone)]
pub struct CdpClient {
    inner: Arc<Inner>,
}

impl fmt::Debug for CdpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CdpClient")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl CdpClient {
    pub fn from_pipe<R, W>(reader: R, writer: W, child: Option<Child>) -> Result<Self, CdpError>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let (inner, writer_rx) = new_inner(child);

        let writer_inner = Arc::downgrade(&inner);
        let writer_thread = thread::Builder::new()
            .name("scrapling-cdp-writer".to_string())
            .spawn(move || writer_loop(writer, writer_rx, writer_inner))
            .map_err(|error| CdpError::Transport(error.to_string()))?;
        *lock(&inner.writer_thread) = Some(writer_thread);

        let reader_inner = Arc::downgrade(&inner);
        let reader_thread = match thread::Builder::new()
            .name("scrapling-cdp-reader".to_string())
            .spawn(move || reader_loop(reader, reader_inner))
        {
            Ok(thread) => thread,
            Err(error) => {
                inner.terminate(CdpError::Transport(error.to_string()));
                if let Some(thread) = lock(&inner.writer_thread).take() {
                    let _ = thread.join();
                }
                return Err(CdpError::Transport(error.to_string()));
            }
        };
        *lock(&inner.reader_thread) = Some(reader_thread);
        Ok(Self { inner })
    }

    #[cfg(unix)]
    /// Spawn one Chrome process with CDP mapped to fd3/fd4.
    ///
    /// `CommandExt::pre_exec` has no removal API, so `command` is single-use
    /// after this call, including when spawning fails.
    pub fn launch_pipe(command: &mut Command) -> Result<Self, CdpError> {
        use std::fs::File;
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;

        let (child_commands, parent_commands) = pipe_cloexec()?;
        let (parent_events, child_events) = pipe_cloexec()?;
        let child_commands = dup_cloexec(child_commands.as_raw_fd())?;
        let child_events = dup_cloexec(child_events.as_raw_fd())?;
        let command_fd = child_commands.as_raw_fd();
        let event_fd = child_events.as_raw_fd();

        if !command
            .get_args()
            .any(|argument| argument == REMOTE_DEBUGGING_PIPE_ARG)
        {
            command.arg(REMOTE_DEBUGGING_PIPE_ARG);
        }
        // SAFETY: the closure calls only async-signal-safe libc functions and
        // captures raw integers. Both sources are duplicated above fd 4, so
        // mapping one cannot clobber the other.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(command_fd, 3) == -1 || libc::dup2(event_fd, 4) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command
            .spawn()
            .map_err(|error| CdpError::Transport(format!("launching Chrome: {error}")))?;
        drop(child_commands);
        drop(child_events);
        let reader = File::from(parent_events);
        let writer = File::from(parent_commands);
        Self::from_pipe(reader, writer, Some(child))
    }

    #[cfg(not(unix))]
    pub fn launch_pipe(_command: &mut Command) -> Result<Self, CdpError> {
        Err(CdpError::UnsupportedTransport(
            "--remote-debugging-pipe launch is supported only on Unix".to_string(),
        ))
    }

    pub async fn connect_websocket_url(url: &str) -> Result<Self, CdpError> {
        let scheme = url.split(':').next().unwrap_or_default();
        if !matches!(scheme, "ws" | "wss") {
            return Err(CdpError::UnsupportedTransport(format!(
                "cdp_url must use ws:// or wss://, got '{url}'"
            )));
        }
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| CdpError::Transport(format!("connecting cdp_url '{url}': {error}")))?;
        let (inner, writer_rx) = new_inner(None);
        let task_inner = Arc::downgrade(&inner);
        let task = tokio::spawn(websocket_loop(socket, writer_rx, task_inner));
        *lock(&inner.websocket_task) = Some(task);
        Ok(Self { inner })
    }

    pub fn session(&self, session_id: impl Into<String>) -> CdpSession {
        CdpSession {
            client: self.clone(),
            session_id: session_id.into(),
        }
    }

    pub fn send(&self, method: &str, params: Value) -> Result<CdpCommand, CdpError> {
        self.send_to(None, method, params)
    }

    fn send_to(
        &self,
        session_id: Option<String>,
        method: &str,
        params: Value,
    ) -> Result<CdpCommand, CdpError> {
        if self.is_closed() {
            return Err(CdpError::Closed);
        }
        if method.is_empty() {
            return Err(CdpError::InvalidMessage(
                "CDP method cannot be empty".to_string(),
            ));
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut message = Map::new();
        message.insert("id".to_string(), Value::from(id));
        message.insert("method".to_string(), Value::from(method));
        message.insert("params".to_string(), params);
        if let Some(value) = &session_id {
            message.insert("sessionId".to_string(), Value::from(value.clone()));
        }
        let mut frame = serde_json::to_vec(&Value::Object(message))
            .map_err(|error| CdpError::InvalidMessage(error.to_string()))?;
        if frame.len() > MAX_CDP_MESSAGE_BYTES {
            return Err(CdpError::InvalidMessage(format!(
                "CDP message exceeds {MAX_CDP_MESSAGE_BYTES} bytes"
            )));
        }
        frame.push(0);

        let (sender, receiver) = oneshot::channel();
        lock(&self.inner.pending).insert(id, Pending { session_id, sender });
        if self
            .inner
            .writer_tx
            .send(WriterCommand::Frame(frame))
            .is_err()
        {
            lock(&self.inner.pending).remove(&id);
            self.inner.terminate(CdpError::Closed);
            return Err(CdpError::Closed);
        }
        // Close the TOCTOU with terminate(): it may have drained `pending`
        // between the is_closed check above and the insert, leaving this
        // entry to never complete. Re-check after the insert — whichever side
        // runs second cleans up.
        if self.is_closed() {
            lock(&self.inner.pending).remove(&id);
            return Err(CdpError::Closed);
        }
        Ok(CdpCommand {
            id,
            receiver,
            inner: Arc::downgrade(&self.inner),
            completed: false,
            deadline: None,
        })
    }

    pub fn subscribe(&self) -> EventReceiver {
        EventReceiver {
            receiver: self.inner.events.subscribe(),
            disconnected: self.inner.disconnected.subscribe(),
            session_id: None,
            any_session: false,
        }
    }

    /// Subscribe to events from every session (see `EventReceiver::any_session`).
    pub fn subscribe_any(&self) -> EventReceiver {
        EventReceiver {
            receiver: self.inner.events.subscribe(),
            disconnected: self.inner.disconnected.subscribe(),
            session_id: None,
            any_session: true,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    pub fn close(&self) -> Result<(), CdpError> {
        self.inner.terminate(CdpError::Closed);
        join_thread(&self.inner.writer_thread)?;
        join_thread(&self.inner.reader_thread)?;
        Ok(())
    }

    pub fn process_status(&self) -> Option<ExitStatus> {
        lock(&self.inner.process).status
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        lock(&self.inner.pending).len()
    }
}

#[derive(Clone, Debug)]
pub struct CdpSession {
    client: CdpClient,
    session_id: String,
}

impl CdpSession {
    pub fn id(&self) -> &str {
        &self.session_id
    }

    pub fn send(&self, method: &str, params: Value) -> Result<CdpCommand, CdpError> {
        self.client
            .send_to(Some(self.session_id.clone()), method, params)
    }

    pub fn subscribe(&self) -> EventReceiver {
        EventReceiver {
            receiver: self.client.inner.events.subscribe(),
            disconnected: self.client.inner.disconnected.subscribe(),
            session_id: Some(self.session_id.clone()),
            any_session: false,
        }
    }
}

/// Hard ceiling on any single CDP command round-trip. A blocked renderer
/// main thread (`while(1){}` with the hang monitor disabled, a wedged
/// browser process) simply never answers, and an unbounded await here is
/// how one-shot fetches hang and session actors wedge forever. Generous on
/// purpose: real commands answer in milliseconds and even a tarpit
/// navigation is bounded by Chrome's own ~5-minute network timeout region.
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

pub struct CdpCommand {
    id: u64,
    receiver: oneshot::Receiver<Result<Value, CdpError>>,
    inner: Weak<Inner>,
    completed: bool,
    deadline: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Future for CdpCommand {
    type Output = Result<Value, CdpError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(context) {
            Poll::Ready(Ok(result)) => {
                self.completed = true;
                Poll::Ready(result)
            }
            Poll::Ready(Err(_)) => {
                self.completed = true;
                Poll::Ready(Err(CdpError::Closed))
            }
            Poll::Pending => {
                let deadline = self
                    .deadline
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(COMMAND_TIMEOUT)));
                match deadline.as_mut().poll(context) {
                    Poll::Ready(()) => {
                        self.completed = true;
                        if let Some(inner) = self.inner.upgrade() {
                            lock(&inner.pending).remove(&self.id);
                        }
                        Poll::Ready(Err(CdpError::Transport(format!(
                            "CDP command timed out after {}s (renderer or browser unresponsive)",
                            COMMAND_TIMEOUT.as_secs()
                        ))))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

impl Drop for CdpCommand {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Some(inner) = self.inner.upgrade() {
            lock(&inner.pending).remove(&self.id);
        }
    }
}

fn new_inner(child: Option<Child>) -> (Arc<Inner>, mpsc::UnboundedReceiver<WriterCommand>) {
    let (writer_tx, writer_rx) = mpsc::unbounded_channel();
    // Sized for bursty pages: every session's events share this channel, and
    // an overflow only costs the laggard skipped events (recv treats Lagged
    // as recoverable), but skipping is still worth avoiding.
    let (events, _) = broadcast::channel(4096);
    let (disconnected, _) = watch::channel(false);
    let inner = Arc::new(Inner {
        next_id: AtomicU64::new(0),
        closed: AtomicBool::new(false),
        pending: Mutex::new(HashMap::new()),
        events,
        disconnected,
        writer_tx,
        writer_thread: Mutex::new(None),
        reader_thread: Mutex::new(None),
        websocket_task: Mutex::new(None),
        process: Mutex::new(ProcessState {
            child,
            status: None,
        }),
    });
    (inner, writer_rx)
}

fn writer_loop<W: Write>(
    mut writer: W,
    mut commands: mpsc::UnboundedReceiver<WriterCommand>,
    inner: Weak<Inner>,
) {
    while let Some(command) = commands.blocking_recv() {
        match command {
            WriterCommand::Frame(frame) => {
                if let Err(error) = writer.write_all(&frame).and_then(|()| writer.flush()) {
                    if let Some(inner) = inner.upgrade() {
                        inner.terminate(CdpError::Transport(format!("writing CDP pipe: {error}")));
                    }
                    break;
                }
            }
            WriterCommand::Shutdown => break,
        }
    }
}

async fn websocket_loop<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    mut commands: mpsc::UnboundedReceiver<WriterCommand>,
    inner: Weak<Inner>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut writer, mut reader) = socket.split();
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(WriterCommand::Frame(mut frame)) => {
                    if frame.last() == Some(&0) {
                        frame.pop();
                    }
                    let text = match String::from_utf8(frame) {
                        Ok(text) => text,
                        Err(error) => {
                            terminate_weak(&inner, CdpError::InvalidMessage(error.to_string()));
                            break;
                        }
                    };
                    if let Err(error) = writer.send(Message::Text(text.into())).await {
                        terminate_weak(&inner, CdpError::Transport(format!("writing cdp_url: {error}")));
                        break;
                    }
                }
                Some(WriterCommand::Shutdown) | None => {
                    let _ = writer.send(Message::Close(None)).await;
                    let _ = writer.close().await;
                    break;
                }
            },
            incoming = reader.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if !route_websocket_message(&inner, text.as_bytes()) {
                        break;
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if !route_websocket_message(&inner, &bytes) {
                        break;
                    }
                }
                Some(Ok(Message::Ping(bytes))) => {
                    if let Err(error) = writer.send(Message::Pong(bytes)).await {
                        terminate_weak(&inner, CdpError::Transport(format!("writing cdp_url pong: {error}")));
                        break;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => {
                    terminate_weak(&inner, CdpError::Closed);
                    break;
                }
                Some(Ok(Message::Frame(_))) => {}
                Some(Err(error)) => {
                    terminate_weak(&inner, CdpError::Transport(format!("reading cdp_url: {error}")));
                    break;
                }
            }
        }
    }
}

fn route_websocket_message(inner: &Weak<Inner>, bytes: &[u8]) -> bool {
    let message = match serde_json::from_slice(bytes) {
        Ok(message) => message,
        Err(error) => {
            terminate_weak(
                inner,
                CdpError::InvalidMessage(format!("invalid CDP JSON: {error}")),
            );
            return false;
        }
    };
    let Some(inner) = inner.upgrade() else {
        return false;
    };
    match inner.route(message) {
        Ok(()) => true,
        Err(error) => {
            inner.terminate(error);
            false
        }
    }
}

fn terminate_weak(inner: &Weak<Inner>, error: CdpError) {
    if let Some(inner) = inner.upgrade() {
        inner.terminate(error);
    }
}

fn reader_loop<R: Read>(reader: R, inner: Weak<Inner>) {
    let mut frames = NulFrames::new(reader);
    loop {
        let result = match frames.next() {
            Ok(Some(frame)) => serde_json::from_slice(&frame)
                .map_err(|error| CdpError::InvalidMessage(format!("invalid CDP JSON: {error}"))),
            Ok(None) => {
                if let Some(inner) = inner.upgrade() {
                    inner.terminate(CdpError::Closed);
                }
                return;
            }
            Err(error) => Err(error),
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        match result.and_then(|message| inner.route(message)) {
            Ok(()) => {}
            Err(error) => {
                inner.terminate(error);
                return;
            }
        }
    }
}

struct NulFrames<R> {
    reader: R,
    pending: Vec<u8>,
}

impl<R: Read> NulFrames<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
        }
    }

    fn next(&mut self) -> Result<Option<Vec<u8>>, CdpError> {
        loop {
            if let Some(end) = self.pending.iter().position(|byte| *byte == 0) {
                if end > MAX_CDP_MESSAGE_BYTES {
                    return Err(CdpError::InvalidMessage(format!(
                        "CDP message exceeds {MAX_CDP_MESSAGE_BYTES} bytes"
                    )));
                }
                let mut remainder = self.pending.split_off(end + 1);
                std::mem::swap(&mut remainder, &mut self.pending);
                remainder.pop();
                return Ok(Some(remainder));
            }
            if self.pending.len() > MAX_CDP_MESSAGE_BYTES {
                return Err(CdpError::InvalidMessage(format!(
                    "CDP message exceeds {MAX_CDP_MESSAGE_BYTES} bytes"
                )));
            }
            let mut buffer = [0; 8192];
            match self.reader.read(&mut buffer) {
                Ok(0) if self.pending.is_empty() => return Ok(None),
                Ok(0) => {
                    return Err(CdpError::InvalidMessage(
                        "CDP pipe closed during a frame".to_string(),
                    ));
                }
                Ok(count) => self.pending.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(CdpError::Transport(format!("reading CDP pipe: {error}")));
                }
            }
        }
    }
}

fn join_thread(slot: &Mutex<Option<JoinHandle<()>>>) -> Result<(), CdpError> {
    if let Some(thread) = lock(slot).take() {
        thread
            .join()
            .map_err(|_| CdpError::Transport("CDP transport thread panicked".to_string()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn pipe_cloexec() -> Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd), CdpError> {
    use std::os::fd::FromRawFd;

    let mut descriptors = [-1; 2];
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: `descriptors` points to space for exactly two file descriptors.
    let result = unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    // SAFETY: `descriptors` points to space for exactly two file descriptors.
    let result = unsafe { libc::pipe(descriptors.as_mut_ptr()) };
    if result == -1 {
        return Err(CdpError::Transport(format!(
            "creating Chrome CDP pipe: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: successful `pipe2` returned two new, uniquely owned descriptors.
    let pipes = unsafe {
        (
            std::os::fd::OwnedFd::from_raw_fd(descriptors[0]),
            std::os::fd::OwnedFd::from_raw_fd(descriptors[1]),
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    for descriptor in [&pipes.0, &pipes.1] {
        use std::os::fd::AsRawFd;

        // SAFETY: the descriptor is owned and live for this call.
        if unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(CdpError::Transport(format!(
                "setting close-on-exec on Chrome CDP pipe: {}",
                std::io::Error::last_os_error()
            )));
        }
    }
    Ok(pipes)
}

#[cfg(unix)]
fn dup_cloexec(descriptor: std::os::fd::RawFd) -> Result<std::os::fd::OwnedFd, CdpError> {
    use std::os::fd::FromRawFd;

    // Keep both pre-exec source descriptors above Chrome's fixed fd3/fd4.
    // SAFETY: `descriptor` is live for this call and `fcntl` creates a new fd.
    let duplicated = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 5) };
    if duplicated == -1 {
        return Err(CdpError::Transport(format!(
            "duplicating Chrome CDP pipe: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: successful `fcntl(F_DUPFD_CLOEXEC)` returned a new owned fd.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(duplicated) })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
