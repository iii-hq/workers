// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! sandbox::fs::write — streaming file upload trigger.
//!
//! The caller creates a channel and passes its `reader_ref` (a
//! `StreamChannelRef`) in the request JSON. This handler constructs a
//! `ChannelReader` from that ref, adapts it to `tokio::io::AsyncRead`
//! via `ChannelReaderAdapter`, then streams bytes into the supervisor
//! through `FsRunner::fs_write_stream`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use iii_sdk::RegisterFunction;
use iii_sdk::channels::{ChannelReader, StreamChannelRef};
use iii_shell_proto::FsResult;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sandbox_daemon::{
    errors::{SandboxError, SandboxErrorWire},
    fs::adapter::FsRunner,
    registry::SandboxRegistry,
};

// ---------------------------------------------------------------------------
// AsyncRead adapter for ChannelReader
// ---------------------------------------------------------------------------

/// Wraps a `ChannelReader` and implements `tokio::io::AsyncRead` by driving
/// `next_binary()` on each poll. Because `ChannelReader::next_binary` is
/// async and `poll_read` is synchronous, we store a pending future when the
/// current chunk isn't ready yet. Leftover bytes from a chunk larger than
/// the read buffer are kept in `pending_buf`.
///
/// # Limitations
/// This adapter holds a `tokio::runtime::Handle` internally and spawns the
/// async `next_binary()` call as a task, then polls the JoinHandle. This
/// keeps the `AsyncRead` impl `Unpin` without unsafe and avoids storing a
/// `Pin<Box<dyn Future>>` inline (which would require `unsafe Unpin`).
pub struct ChannelReaderAdapter {
    /// Starts the read of the next frame: `ChannelReader::next_binary`, or a
    /// scripted source in tests.
    next: Box<dyn Fn() -> BoxFuture<'static, Frame> + Send>,
    pending_buf: Vec<u8>,
    pending_task: Option<tokio::task::JoinHandle<Frame>>,
    eof: bool,
}

/// One `next_binary` result: a frame, `None` at the end of the stream.
type Frame = Result<Option<Vec<u8>>, iii_sdk::Error>;

impl ChannelReaderAdapter {
    pub fn new(reader: ChannelReader) -> Self {
        let reader = Arc::new(reader);
        Self::from_source(move || {
            let reader = reader.clone();
            Box::pin(async move { reader.next_binary().await })
        })
    }

    fn from_source(next: impl Fn() -> BoxFuture<'static, Frame> + Send + 'static) -> Self {
        Self {
            next: Box::new(next),
            pending_buf: Vec::new(),
            pending_task: None,
            eof: false,
        }
    }
}

/// A dropped upload (S218 on the caller's idle timeout) must not leave its
/// spawned read holding the channel until the caller's writer closes.
impl Drop for ChannelReaderAdapter {
    fn drop(&mut self) {
        if let Some(task) = self.pending_task.take() {
            task.abort();
        }
    }
}

impl tokio::io::AsyncRead for ChannelReaderAdapter {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        if this.eof && this.pending_buf.is_empty() {
            return Poll::Ready(Ok(()));
        }

        // Drain any leftover bytes from a previous oversized chunk.
        if !this.pending_buf.is_empty() {
            let n = this.pending_buf.len().min(buf.remaining());
            buf.put_slice(&this.pending_buf[..n]);
            this.pending_buf.drain(..n);
            return Poll::Ready(Ok(()));
        }

        loop {
            // Spawn a task if we don't have one in flight.
            let task = this
                .pending_task
                .get_or_insert_with(|| tokio::spawn((this.next)()));

            // Poll the in-flight task.
            let join_result = std::task::ready!(Pin::new(task).poll(cx));
            this.pending_task = None;
            let chunk_result = join_result.map_err(|e| {
                io::Error::new(io::ErrorKind::BrokenPipe, format!("channel task: {e}"))
            })?;
            return match chunk_result {
                Ok(None) => {
                    this.eof = true;
                    Poll::Ready(Ok(()))
                }
                // An empty frame (the Python SDK sends one for `write(b"")`) is
                // no data: returning it would fill 0 bytes, which reads as EOF
                // and commits a truncated file.
                Ok(Some(data)) if data.is_empty() => continue,
                Ok(Some(data)) => {
                    let n = data.len().min(buf.remaining());
                    buf.put_slice(&data[..n]);
                    if n < data.len() {
                        this.pending_buf.extend_from_slice(&data[n..]);
                    }
                    Poll::Ready(Ok(()))
                }
                Err(e) => Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel read error: {e}"),
                ))),
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// File body for `sandbox::fs::write`. Untagged so the JSON shape
/// decides which variant runs:
///
/// - A bare JSON string (`"console.log('hi')"`) → [`WriteContent::Utf8`].
///   This is what LLM agents naturally pass and the recommended form
///   for source files / configs / small text.
/// - An object that matches [`StreamChannelRef`] → [`WriteContent::Stream`].
///   The existing channel-streaming path for large or binary payloads
///   coming from a programmatic caller that can construct a channel.
///
/// Serde tries variants in declaration order; `Utf8` matches first
/// because every JSON string deserialises into `String`, and
/// `StreamChannelRef` requires an object. Binary inline data uses the
/// separate `content_b64` field on [`WriteRequest`] (not a variant
/// here, so a caller can't accidentally pass base64 expecting it to be
/// decoded — they have to opt in by name).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum WriteContent {
    /// Inline UTF-8 string. Written to the file verbatim.
    Utf8(String),
    /// Streaming channel for large / binary uploads.
    Stream(StreamChannelRef),
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(example = "write_request_example")]
pub struct WriteRequest {
    /// UUID returned by `sandbox::create`.
    pub sandbox_id: String,
    /// Absolute destination path inside the sandbox guest.
    pub path: String,
    /// Octal permissions for the new file (default `"0644"`).
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Create missing parent directories before writing.
    #[serde(default)]
    pub parents: bool,
    /// File body. Pass a UTF-8 string for source/text (the agent-natural
    /// form), or a `StreamChannelRef` object for streaming large/binary
    /// uploads. Mutually exclusive with `content_b64`; exactly one of the
    /// two must be set.
    #[serde(default)]
    pub content: Option<WriteContent>,
    /// Base64-encoded inline body for small binary payloads.
    /// Mutually exclusive with `content`.
    #[serde(default)]
    pub content_b64: Option<String>,
}

fn write_request_example() -> serde_json::Value {
    serde_json::json!({
        "sandbox_id": "00000000-0000-0000-0000-000000000000",
        "path": "/home/app/index.js",
        "content": "console.log('hello world')\n",
        "mode": "0644",
        "parents": true
    })
}

fn default_mode() -> String {
    "0644".to_string()
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WriteResponse {
    pub bytes_written: u64,
    pub path: String,
}

// ---------------------------------------------------------------------------
// Handler — testable inner function
// ---------------------------------------------------------------------------

/// Inner handler that accepts a pre-constructed `AsyncRead`. Tests call
/// this directly with a `Cursor<Vec<u8>>`, bypassing the channel layer.
pub async fn handle_write_with_reader<R: FsRunner + ?Sized>(
    sandbox_id: String,
    path: String,
    mode: String,
    parents: bool,
    reader: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    registry: &SandboxRegistry,
    runner: &R,
) -> Result<WriteResponse, SandboxError> {
    let id = Uuid::parse_str(&sandbox_id).map_err(|_| {
        SandboxError::InvalidRequest(format!("sandbox_id is not a valid UUID: {sandbox_id}"))
    })?;
    let state = registry.get(id).await?;
    if state.stopped {
        return Err(SandboxError::AlreadyStopped(id.to_string()));
    }
    registry.bump_last_exec(id).await;

    let result = runner
        .fs_write_stream(state.shell_sock, path.clone(), mode, parents, reader)
        .await?;

    match result {
        FsResult::Write {
            bytes_written,
            path: p,
        } => Ok(WriteResponse {
            bytes_written,
            path: p,
        }),
        other => Err(SandboxError::FsIo(format!(
            "expected Write result, got {other:?}"
        ))),
    }
}

/// Public trigger handler. Dispatches on the body shape and delegates
/// to [`handle_write_with_reader`].
///
/// Accepts three input shapes (exactly one must be present):
///
/// - `content: "<utf-8 string>"` — inline UTF-8 body, wrapped in a
///   `Cursor` and written verbatim. The path LLM agents naturally take
///   when they pass `content` as a string. No channel setup required.
/// - `content_b64: "<base64>"` — inline binary body. Decoded, then
///   wrapped in a `Cursor` like the UTF-8 form. Use for files with
///   bytes the JSON layer would mangle (NUL, invalid UTF-8, etc.).
/// - `content: { reader_ref: …, … }` — a `StreamChannelRef`. The
///   original streaming path, for large uploads from programmatic
///   callers that can construct a channel.
pub async fn handle_write<R: FsRunner + ?Sized>(
    req: WriteRequest,
    registry: &SandboxRegistry,
    runner: &R,
    engine_address: &str,
) -> Result<WriteResponse, SandboxError> {
    use base64::Engine as _;
    use std::io::Cursor;

    let reader: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match (req.content, req.content_b64)
    {
        (Some(WriteContent::Utf8(s)), None) => Box::new(Cursor::new(s.into_bytes())),
        (Some(WriteContent::Stream(ref_)), None) => {
            let ch_reader = ChannelReader::new(engine_address, &ref_);
            Box::new(ChannelReaderAdapter::new(ch_reader))
        }
        (None, Some(b64)) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .map_err(|e| {
                    SandboxError::InvalidRequest(format!("content_b64 is not valid base64: {e}"))
                })?;
            Box::new(Cursor::new(bytes))
        }
        (Some(_), Some(_)) => {
            return Err(SandboxError::InvalidRequest(
                "set either `content` (string or StreamChannelRef) or `content_b64`, not both"
                    .into(),
            ));
        }
        (None, None) => {
            return Err(SandboxError::InvalidRequest(
                "missing file body: set `content` (string or StreamChannelRef) or `content_b64`"
                    .into(),
            ));
        }
    };

    handle_write_with_reader(
        req.sandbox_id,
        req.path,
        req.mode,
        req.parents,
        reader,
        registry,
        runner,
    )
    .await
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(super) fn register(
    iii: &iii_sdk::IIIClient,
    registry: Arc<SandboxRegistry>,
    runner: Arc<dyn FsRunner>,
) -> iii_sdk::runtime::FunctionRef {
    let engine_address = iii.address().to_string();
    iii.register_function(
        "sandbox-libkrun::fs::write",
        RegisterFunction::new_async(move |req: WriteRequest| {
            let registry = registry.clone();
            let runner = runner.clone();
            let engine_address = engine_address.clone();
            async move {
                let sid = req.sandbox_id.clone();
                let start = std::time::Instant::now();
                let result = handle_write(req, &registry, &*runner, &engine_address).await;
                crate::sandbox_daemon::log_handler_result(
                    "sandbox-libkrun::fs::write",
                    Some(&sid),
                    &result,
                    start.elapsed().as_millis() as u64,
                );
                result.map_err(|e| SandboxErrorWire(e).into())
            }
        })
        .description(
            "Write a file into a sandbox. `content` accepts a UTF-8 string (recommended for \
             source/text), a StreamChannelRef object (for large uploads), or use `content_b64` \
             for small binary. Example: { sandbox_id: \"...\", path: \"/home/app/index.js\", content: \"console.log('hi')\\n\" }",
        ),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_daemon::{fs::adapter::FsRunner, registry::SandboxState};
    use iii_shell_proto::{FsOp, FsReadMeta, FsResult};
    use std::io::Cursor;
    use std::path::PathBuf;

    use tokio::sync::Mutex;

    struct FakeRunner {
        captured: Arc<Mutex<Vec<u8>>>,
    }

    #[async_trait::async_trait]
    impl FsRunner for FakeRunner {
        async fn fs_call(&self, _shell_sock: PathBuf, _op: FsOp) -> Result<FsResult, SandboxError> {
            unimplemented!()
        }

        async fn fs_write_stream(
            &self,
            _shell_sock: PathBuf,
            path: String,
            _mode: String,
            _parents: bool,
            mut reader: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        ) -> Result<FsResult, SandboxError> {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            reader.read_to_end(&mut buf).await.unwrap();
            let bytes = buf.len() as u64;
            *self.captured.lock().await = buf;
            Ok(FsResult::Write {
                bytes_written: bytes,
                path,
            })
        }

        async fn fs_read_stream(
            &self,
            _shell_sock: PathBuf,
            _path: String,
        ) -> Result<(FsReadMeta, Box<dyn tokio::io::AsyncRead + Unpin + Send>), SandboxError>
        {
            unimplemented!()
        }
    }

    fn make_state(id: Uuid) -> SandboxState {
        crate::sandbox_daemon::registry::sandbox_state_for_test(id)
    }

    #[tokio::test]
    async fn write_with_reader_captures_bytes() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(make_state(id)).await;
        let data = b"hello, sandbox!";
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner {
            captured: captured.clone(),
        };
        let cursor = Box::new(Cursor::new(data.to_vec()));
        let resp = handle_write_with_reader(
            id.to_string(),
            "/workspace/test.txt".into(),
            "0644".into(),
            false,
            cursor,
            &reg,
            &runner,
        )
        .await
        .unwrap();
        assert_eq!(resp.bytes_written, data.len() as u64);
        assert_eq!(*captured.lock().await, data);
    }

    #[tokio::test]
    async fn bad_uuid_returns_s001() {
        let reg = SandboxRegistry::new();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { captured };
        let err = handle_write_with_reader(
            "not-a-uuid".into(),
            "/".into(),
            "0644".into(),
            false,
            Box::new(Cursor::new(vec![])),
            &reg,
            &runner,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code().as_str(), "S001");
    }

    #[tokio::test]
    async fn missing_sandbox_returns_s002() {
        let reg = SandboxRegistry::new();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { captured };
        let err = handle_write_with_reader(
            Uuid::new_v4().to_string(),
            "/".into(),
            "0644".into(),
            false,
            Box::new(Cursor::new(vec![])),
            &reg,
            &runner,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code().as_str(), "S002");
    }

    // ── public handle_write — body-shape dispatch (option A) ────────────

    /// `content: "<utf-8 string>"` is the path agents naturally take.
    /// It must just work — no channel setup, no base64, write the bytes.
    #[tokio::test]
    async fn handle_write_accepts_inline_utf8_content() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(make_state(id)).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner {
            captured: captured.clone(),
        };
        let req = WriteRequest {
            sandbox_id: id.to_string(),
            path: "/workspace/app.js".into(),
            mode: "0644".into(),
            parents: false,
            content: Some(WriteContent::Utf8("console.log('hi')\n".into())),
            content_b64: None,
        };
        let resp = handle_write(req, &reg, &runner, "irrelevant")
            .await
            .unwrap();
        assert_eq!(resp.bytes_written, "console.log('hi')\n".len() as u64);
        assert_eq!(*captured.lock().await, b"console.log('hi')\n");
    }

    /// `content_b64` is the inline path for binary or shell-fragile bytes.
    #[tokio::test]
    async fn handle_write_accepts_inline_content_b64() {
        use base64::Engine as _;
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(make_state(id)).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner {
            captured: captured.clone(),
        };
        let payload = b"\x00\x01\x02binary\xffbytes";
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let req = WriteRequest {
            sandbox_id: id.to_string(),
            path: "/workspace/blob.bin".into(),
            mode: "0644".into(),
            parents: false,
            content: None,
            content_b64: Some(b64),
        };
        let resp = handle_write(req, &reg, &runner, "irrelevant")
            .await
            .unwrap();
        assert_eq!(resp.bytes_written, payload.len() as u64);
        assert_eq!(&*captured.lock().await, payload);
    }

    #[tokio::test]
    async fn handle_write_rejects_invalid_base64() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(make_state(id)).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { captured };
        let req = WriteRequest {
            sandbox_id: id.to_string(),
            path: "/x".into(),
            mode: "0644".into(),
            parents: false,
            content: None,
            content_b64: Some("not!valid!base64!".into()),
        };
        let err = handle_write(req, &reg, &runner, "irrelevant")
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S001");
        assert!(err.to_string().contains("not valid base64"), "got: {err}");
    }

    #[tokio::test]
    async fn handle_write_rejects_both_content_and_content_b64() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(make_state(id)).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { captured };
        let req = WriteRequest {
            sandbox_id: id.to_string(),
            path: "/x".into(),
            mode: "0644".into(),
            parents: false,
            content: Some(WriteContent::Utf8("a".into())),
            content_b64: Some("YQ==".into()),
        };
        let err = handle_write(req, &reg, &runner, "irrelevant")
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S001");
        assert!(err.to_string().contains("not both"), "got: {err}");
    }

    #[tokio::test]
    async fn handle_write_rejects_neither_content_nor_content_b64() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(make_state(id)).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { captured };
        let req = WriteRequest {
            sandbox_id: id.to_string(),
            path: "/x".into(),
            mode: "0644".into(),
            parents: false,
            content: None,
            content_b64: None,
        };
        let err = handle_write(req, &reg, &runner, "irrelevant")
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S001");
        assert!(err.to_string().contains("missing file body"), "got: {err}");
    }

    /// A channel whose reads return `frames`, then the end of the stream.
    fn scripted(frames: &'static [&'static [u8]]) -> ChannelReaderAdapter {
        let frames = std::sync::Mutex::new(frames.iter().copied());
        ChannelReaderAdapter::from_source(move || {
            let frame = frames.lock().unwrap().next().map(<[u8]>::to_vec);
            Box::pin(async move { Ok(frame) })
        })
    }

    /// An empty frame (the Python SDK sends one for `write(b"")`) is no data,
    /// not the end: the upload must go on past it.
    #[tokio::test]
    async fn an_empty_frame_does_not_end_the_upload() {
        use tokio::io::AsyncReadExt;
        let mut got = Vec::new();
        scripted(&[b"ab", b"", b"cd"])
            .read_to_end(&mut got)
            .await
            .unwrap();
        assert_eq!(got, b"abcd");
    }

    /// Dropping the adapter mid-upload ends its pending channel read.
    #[tokio::test]
    async fn dropping_the_adapter_ends_its_pending_read() {
        use tokio::io::AsyncReadExt;
        let (held, released) = tokio::sync::oneshot::channel::<()>();
        let held = std::sync::Mutex::new(Some(held));
        let mut reader = ChannelReaderAdapter::from_source(move || {
            let held = held.lock().unwrap().take();
            Box::pin(async move {
                // Released only when this read ends.
                let _held = held;
                std::future::pending::<Frame>().await
            })
        });
        let mut buf = [0u8; 8];
        let read = reader.read(&mut buf);
        let short = std::time::Duration::from_millis(50);
        assert!(tokio::time::timeout(short, read).await.is_err());

        drop(reader);
        let ended = tokio::time::timeout(std::time::Duration::from_secs(1), released).await;
        assert!(ended.is_ok(), "the read outlived the adapter");
    }

    /// Serde dispatch: a bare JSON string lands as `Utf8`; an object
    /// matching `StreamChannelRef` lands as `Stream`. We don't test the
    /// `Stream` path end-to-end here (it requires a live engine for
    /// `ChannelReader`); the unit test covers the JSON → enum shape.
    #[tokio::test]
    async fn write_content_deserialises_string_as_utf8_variant() {
        let v: WriteContent = serde_json::from_value(serde_json::json!("hello"))
            .expect("string must deserialise as Utf8");
        match v {
            WriteContent::Utf8(s) => assert_eq!(s, "hello"),
            other => panic!("expected Utf8, got {other:?}"),
        }
    }
}
