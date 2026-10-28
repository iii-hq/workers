//! R3 — fs::read byte-loss boundary test. `handle_read` buffers up to
//! 1 MiB and tries a UTF-8 decode. If decode fails, or the reader holds
//! more than `meta.size` claimed, the buffered bytes plus any remaining
//! stream contents go out through a channel. These tests drive the real
//! `handle_read` against a loopback engine and compare every byte the
//! channel delivers with the file.

use std::path::PathBuf;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
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
use tokio::io::AsyncRead;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
};
use uuid::Uuid;

const INLINE_BUFFER_CAP: usize = 1024 * 1024;

struct FakeRunnerStream {
    body: Vec<u8>,
    meta_size: u64,
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
        Ok((meta, Box::new(cursor)))
    }
}

/// What a channel's writer sent before it closed.
#[derive(Default)]
struct Written {
    bytes: Vec<u8>,
    messages: Vec<String>,
}

/// A loopback engine just big enough for `handle_read`: it answers
/// `engine::channels::create` and hands the test each channel's writes once
/// its writer closes.
async fn fake_engine() -> (IIIClient, mpsc::UnboundedReceiver<Written>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (written, received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(connection(stream, written.clone()));
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
    (iii, received)
}

// Tungstenite's handshake callback fixes this error type.
#[expect(clippy::result_large_err)]
async fn connection(stream: TcpStream, written: mpsc::UnboundedSender<Written>) {
    let mut path = String::new();
    let ws = tokio_tungstenite::accept_hdr_async(stream, |req: &Request, resp: Response| {
        path = req.uri().to_string();
        Ok(resp)
    })
    .await
    .unwrap();
    let (mut sink, mut source) = ws.split();
    if path.starts_with("/ws/channels/") {
        let mut out = Written::default();
        while let Some(Ok(message)) = source.next().await {
            match message {
                Message::Binary(bytes) => out.bytes.extend_from_slice(&bytes),
                Message::Text(text) => out.messages.push(text.to_string()),
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = written.send(out);
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

/// Runs `handle_read` over `body` (with the supervisor claiming `meta_size`)
/// and returns the response plus everything its channel delivered.
async fn read_through_channel(body: Vec<u8>, meta_size: u64) -> (ReadResponse, Written) {
    let (iii, mut written) = fake_engine().await;
    let reg = SandboxRegistry::new();
    let id = Uuid::new_v4();
    reg.insert(sandbox_libkrun::sandbox_daemon::registry::sandbox_state_for_test(id))
        .await;
    let runner = FakeRunnerStream { body, meta_size };
    let req = ReadRequest {
        sandbox_id: id.to_string(),
        path: "/tmp/file".into(),
    };
    let resp = handle_read(req, &reg, &runner, &iii).await.unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(10), written.recv())
        .await
        .expect("channel writer closed")
        .unwrap();
    assert!(
        delivered.messages.is_empty(),
        "pump reported an error: {:?}",
        delivered.messages
    );
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
