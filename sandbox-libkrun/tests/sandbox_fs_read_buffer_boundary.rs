//! R3 — fs::read byte-loss boundary test. `handle_read` buffers up to
//! 1 MiB and tries a UTF-8 decode. If decode fails, or the reader holds
//! more than `meta.size` claimed, the buffered bytes plus any remaining
//! stream contents go out through a channel. These tests drive the real
//! `handle_read` against a loopback engine and compare every byte a real
//! `ChannelReader` gets with the file, and how the stream ends for it.

use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use iii_sdk::channels::ChannelReader;
use iii_sdk::runtime::IIIConnectionState;
use iii_sdk::{IIIClient, InitOptions, register_worker};
use iii_shell_proto::{FsOp, FsReadMeta, FsResult};
use sandbox_libkrun::sandbox_daemon::{
    errors::SandboxError,
    fs::adapter::FsRunner,
    fs::read::{ReadRequest, ReadResponse, handle_read},
    registry::SandboxRegistry,
};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
    protocol::{CloseFrame, frame::coding::CloseCode},
};
use uuid::Uuid;

const INLINE_BUFFER_CAP: usize = 1024 * 1024;

struct FakeRunnerStream {
    body: Vec<u8>,
    meta_size: u64,
    /// After `body` the read fails, as when the VM is killed under it.
    cut: bool,
}

struct CutReader;

impl AsyncRead for CutReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()))
    }
}

#[async_trait::async_trait]
impl FsRunner for FakeRunnerStream {
    async fn fs_call(&self, _shell_sock: PathBuf, _op: FsOp) -> Result<FsResult, SandboxError> {
        unimplemented!("only fs_read_stream is exercised by R3");
    }
    async fn fs_write_stream(
        &self,
        _shell_sock: PathBuf,
        _path: String,
        _mode: String,
        _parents: bool,
        _reader: Box<dyn AsyncRead + Unpin + Send>,
    ) -> Result<FsResult, SandboxError> {
        unimplemented!();
    }
    async fn fs_read_stream(
        &self,
        _shell_sock: PathBuf,
        _path: String,
    ) -> Result<(FsReadMeta, Box<dyn AsyncRead + Unpin + Send>), SandboxError> {
        let meta = FsReadMeta {
            size: self.meta_size,
            mode: "0644".into(),
            mtime: 0,
        };
        let cursor = std::io::Cursor::new(self.body.clone());
        if self.cut {
            return Ok((meta, Box::new(cursor.chain(CutReader))));
        }
        Ok((meta, Box::new(cursor)))
    }
}

/// What a channel's reader got, and how the stream ended for it.
struct Delivered {
    bytes: Vec<u8>,
    messages: Vec<String>,
    end: Result<(), String>,
}

/// The loopback engine's channel: one, from writer to reader.
struct Engine {
    to_reader: mpsc::UnboundedSender<Message>,
    from_writer: tokio::sync::Mutex<mpsc::UnboundedReceiver<Message>>,
    refuse_writer_dial: AtomicBool,
}

/// A loopback engine just big enough for `handle_read`: it answers
/// `engine::channels::create` and relays the channel from writer to reader.
async fn fake_engine(refuse_writer_dial: bool) -> IIIClient {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (to_reader, from_writer) = mpsc::unbounded_channel();
    let engine = Arc::new(Engine {
        to_reader,
        from_writer: from_writer.into(),
        refuse_writer_dial: refuse_writer_dial.into(),
    });
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(connection(stream, engine.clone()));
        }
    });
    let iii = register_worker(&url, InitOptions::default());
    tokio::time::timeout(Duration::from_secs(5), async {
        while iii.get_connection_state() != IIIConnectionState::Connected {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("loopback engine handshake");
    iii
}

// Tungstenite's handshake callback fixes this error type.
#[expect(clippy::result_large_err)]
async fn connection(stream: TcpStream, engine: Arc<Engine>) {
    let mut path = String::new();
    let ws = tokio_tungstenite::accept_hdr_async(stream, |req: &Request, resp: Response| {
        path = req.uri().to_string();
        if path.contains("dir=write") && engine.refuse_writer_dial.swap(false, Ordering::SeqCst) {
            return Err(Response::builder().status(503).body(None).unwrap());
        }
        Ok(resp)
    })
    .await;
    let Ok(ws) = ws else { return };
    let (mut sink, mut source) = ws.split();
    if path.contains("dir=write") {
        while let Some(Ok(message)) = source.next().await {
            let closed = message.is_close();
            let _ = engine.to_reader.send(message);
            if closed {
                return;
            }
        }
        // As the engine does: a writer socket that ends with no close frame
        // reaches the reader as 1011.
        let _ = engine.to_reader.send(Message::Close(Some(CloseFrame {
            code: CloseCode::Error,
            reason: "channel writer aborted: connection lost before close".into(),
        })));
        return;
    }
    if path.contains("dir=read") {
        let mut from_writer = engine.from_writer.lock().await;
        while let Some(message) = from_writer.recv().await {
            let closed = message.is_close();
            if sink.send(message).await.is_err() || closed {
                return;
            }
        }
        return;
    }
    while let Some(Ok(message)) = source.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let frame: Value = serde_json::from_str(&text).unwrap();
        if frame["type"] != "invokefunction" || frame["function_id"] != "engine::channels::create" {
            continue;
        }
        let id = Uuid::new_v4().to_string();
        let reply = json!({
            "type": "invocationresult",
            "invocation_id": frame["invocation_id"],
            "function_id": frame["function_id"],
            "result": {
                "writer": {"channel_id": id, "access_key": "k", "direction": "write"},
                "reader": {"channel_id": id, "access_key": "k", "direction": "read"},
            },
        });
        if sink
            .send(Message::Text(reply.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
    }
}

/// Runs `handle_read` with `runner` and reads its channel as a caller does.
async fn read_channel(iii: &IIIClient, runner: FakeRunnerStream) -> (ReadResponse, Delivered) {
    let reg = SandboxRegistry::new();
    let id = Uuid::new_v4();
    reg.insert(sandbox_libkrun::sandbox_daemon::registry::sandbox_state_for_test(id))
        .await;
    let req = ReadRequest {
        sandbox_id: id.to_string(),
        path: "/tmp/file".into(),
    };
    let resp = handle_read(req, &reg, &runner, iii).await.unwrap();
    let reader = ChannelReader::new(iii.address(), &resp.content);
    let messages = Arc::new(Mutex::new(Vec::new()));
    let seen = messages.clone();
    reader
        .on_message(move |message| seen.lock().unwrap().push(message))
        .await;
    let mut bytes = Vec::new();
    let end = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match reader.next_binary().await {
                Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
                Ok(None) => return Ok(()),
                Err(e) => return Err(e.to_string()),
            }
        }
    })
    .await
    .expect("channel ended");
    let messages = messages.lock().unwrap().clone();
    (
        resp,
        Delivered {
            bytes,
            messages,
            end,
        },
    )
}

/// Runs `handle_read` over `body` (with the supervisor claiming `meta_size`)
/// and returns the response plus everything its channel delivered.
async fn read_through_channel(body: Vec<u8>, meta_size: u64) -> (ReadResponse, Delivered) {
    let iii = fake_engine(false).await;
    let runner = FakeRunnerStream {
        body,
        meta_size,
        cut: false,
    };
    let (resp, delivered) = read_channel(&iii, runner).await;
    assert!(
        delivered.messages.is_empty(),
        "pump reported an error: {:?}",
        delivered.messages
    );
    assert_eq!(delivered.end, Ok(()), "a whole file ends cleanly");
    (resp, delivered)
}

/// 999 KiB of mostly ASCII with a 0xFF every 1024 bytes, so UTF-8 decode fails.
fn invalid_utf8_999kib() -> Vec<u8> {
    let buf: Vec<u8> = (0..999 * 1024)
        .map(|i| {
            if i % 1024 == 0 {
                0xFF
            } else {
                b'a' + (i % 26) as u8
            }
        })
        .collect();
    assert!(std::str::from_utf8(&buf).is_err());
    buf
}

#[tokio::test]
async fn invalid_utf8_under_threshold_falls_through_to_stream() {
    let body = invalid_utf8_999kib();
    let (resp, delivered) = read_through_channel(body.clone(), body.len() as u64).await;
    assert!(resp.body.is_none());
    assert_eq!(resp.size, body.len() as u64);
    assert!(
        delivered.bytes == body,
        "channel bytes differ from the file"
    );
}

#[tokio::test]
async fn valid_utf8_under_threshold_returns_utf8_string_invariant() {
    let text = "hello world\n".repeat(100);
    let (resp, delivered) =
        read_through_channel(text.clone().into_bytes(), text.len() as u64).await;
    assert_eq!(resp.body.as_deref(), Some(text.as_str()));
    assert_eq!(delivered.bytes, text.as_bytes());
}

/// The supervisor under-reports the size: the reader runs past the 1 MiB
/// buffer, and the remainder must be chained after it, not dropped.
#[tokio::test]
async fn reader_longer_than_meta_size_delivers_every_byte() {
    for len in [
        INLINE_BUFFER_CAP,
        INLINE_BUFFER_CAP + 1,
        INLINE_BUFFER_CAP + 70_000,
    ] {
        let body: Vec<u8> = (0..len).map(|i| b'a' + (i % 26) as u8).collect();
        let (resp, delivered) = read_through_channel(body.clone(), 10).await;
        assert!(
            resp.body.is_none(),
            "len {len}: no inline body past the cap"
        );
        assert!(
            delivered.bytes == body,
            "len {len}: channel delivered {} of {} bytes",
            delivered.bytes.len(),
            body.len()
        );
    }
}

/// The VM dies mid-read (`stop_all` on a signal does that with the adapter still
/// up): a reader that only takes bytes must get an error, not a short file that
/// ends like a whole one.
#[tokio::test]
async fn read_cut_mid_stream_is_an_error_for_the_reader() {
    let body = vec![b'a'; 200 * 1024];
    let iii = fake_engine(false).await;
    let runner = FakeRunnerStream {
        body: body.clone(),
        meta_size: 64 * 1024 * 1024,
        cut: true,
    };
    let (_, delivered) = read_channel(&iii, runner).await;
    assert!(delivered.bytes == body, "the bytes before the cut arrive");
    assert!(
        delivered.messages.iter().any(|m| m.contains("S218")),
        "{:?}",
        delivered.messages
    );
    let error = delivered.end.expect_err("a cut read must not end cleanly");
    assert!(error.contains("1011"), "{error}");
}

/// The writer's first dial fails, so nothing was sent; reporting that dials
/// again. A clean close on that socket would pass the file off as empty.
#[tokio::test]
async fn failed_first_write_is_an_error_for_the_reader() {
    let iii = fake_engine(true).await;
    let runner = FakeRunnerStream {
        body: vec![b'a'; 1024],
        meta_size: 64 * 1024 * 1024,
        cut: false,
    };
    let (_, delivered) = read_channel(&iii, runner).await;
    assert!(delivered.bytes.is_empty());
    let error = delivered
        .end
        .expect_err("a lost file must not read as empty");
    assert!(error.contains("1011"), "{error}");
}
