//! Real iii stream plumbing for the relay (no abstraction: these are the
//! concrete bridges over `iii_sdk` streams).
//! `create_router_channel` mints a router-owned engine pipe with
//! `engine::channels::create` and keeps both refs: the relay reads the read
//! end as an [`InputStream`], and the write end is either forwarded to a
//! provider (`writer_ref`) or written here (`router::complete`). `open_sink`
//! writes to a forwarded `writer_ref`.
//!
//! A write end is opened as an [`OutputStream`] only when this process writes
//! to it, so a ref handed to a provider is never written (or closed) here.
//! It is kept open past its handler's return: the forwarder task closes it
//! after the last frame it queued.
//!
//! FrameSink::send is sync, bridging to the async OutputStream via an
//! unbounded mpsc + forwarder task: a send failure is observed one frame
//! late, same as the WebSocket behavior in the Node SDK. A pump task reads
//! the read end's text frames into an mpsc; its exit (close/error) drops the
//! sender, which is EOF for the relay.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use iii_sdk::protocol::{ChannelItem, StreamChannelRef};
use iii_sdk::{CallRequest, IIIClient, InputStream, OutputStream, errors::Error};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use crate::chat::relay::{CallerGone, FrameSink, ReadEvent, RelayRead, RouterChannel};

enum SinkMsg {
    Frame(String),
    Close,
}

struct SdkSink {
    tx: mpsc::UnboundedSender<SinkMsg>,
    closed: Arc<AtomicBool>,
}

impl FrameSink for SdkSink {
    fn send(&self, msg: &str) -> Result<(), CallerGone> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CallerGone);
        }
        self.tx
            .send(SinkMsg::Frame(msg.to_string()))
            .map_err(|_| CallerGone)
    }
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.tx.send(SinkMsg::Close); // forwarder closes the writer
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// Opens the write end `r` as an [`OutputStream`] bound to `iii`, kept open
/// past the current handler: its forwarder closes it.
fn open_output(iii: &IIIClient, r: &StreamChannelRef) -> Result<OutputStream, Error> {
    let value = serde_json::to_value(r).map_err(|e| Error::Serde(e.to_string()))?;
    let output: OutputStream =
        serde_json::from_value(value).map_err(|e| Error::Serde(e.to_string()))?;
    output.bind(iii);
    output.keep_open();
    Ok(output)
}

/// Spawn the forwarder task that writes to the write end `r`. The end is
/// opened on the first frame or close; a sink dropped unused leaves it
/// untouched.
fn spawn_writer_forwarder(iii: IIIClient, r: StreamChannelRef) -> Arc<SdkSink> {
    let (tx, mut rx) = mpsc::unbounded_channel::<SinkMsg>();
    let closed = Arc::new(AtomicBool::new(false));
    let closed_for_task = closed.clone();
    tokio::spawn(async move {
        let mut writer: Option<OutputStream> = None;
        while let Some(msg) = rx.recv().await {
            if writer.is_none() {
                match open_output(&iii, &r) {
                    Ok(output) => writer = Some(output),
                    Err(error) => {
                        tracing::warn!(%error, "router stream: opening the write end failed");
                        closed_for_task.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            }
            let Some(output) = writer.as_ref() else { break };
            match msg {
                SinkMsg::Frame(m) => {
                    if output.send_text(&m).await.is_err() {
                        closed_for_task.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                SinkMsg::Close => break,
            }
        }
        if let Some(output) = writer {
            let _ = output.close().await;
        }
    });
    Arc::new(SdkSink { tx, closed })
}

struct SdkReader {
    rx: mpsc::UnboundedReceiver<String>,
    close_flag: Arc<AtomicBool>,
    cancel: Arc<Notify>,
}

#[async_trait::async_trait]
impl RelayRead for SdkReader {
    async fn next(&mut self, timeout: Duration) -> ReadEvent {
        if self.close_flag.load(Ordering::SeqCst) {
            return ReadEvent::Eof;
        }
        tokio::select! {
            msg = self.rx.recv() => match msg {
                Some(m) => ReadEvent::Msg(m),
                None => ReadEvent::Eof,
            },
            _ = self.cancel.notified() => ReadEvent::Eof,
            _ = tokio::time::sleep(timeout) => ReadEvent::Timeout,
        }
    }
    fn closer(&self) -> Arc<dyn Fn() + Send + Sync> {
        let flag = self.close_flag.clone();
        let cancel = self.cancel.clone();
        Arc::new(move || {
            flag.store(true, Ordering::SeqCst);
            cancel.notify_waiters();
        })
    }
    fn close(&self) {
        (self.closer())();
    }
}

/// The `(writer, reader)` refs of a new engine pipe.
async fn create_pipe_refs(iii: &IIIClient) -> Result<(StreamChannelRef, StreamChannelRef), Error> {
    let mut result = iii
        .call(CallRequest::new(
            "engine::channels::create",
            json!({ "buffer_size": Value::Null }),
        ))
        .await?;
    let mut end = |key: &str| -> Result<StreamChannelRef, Error> {
        let value = result
            .get_mut(key)
            .map(Value::take)
            .ok_or_else(|| Error::Serde(format!("missing '{key}' in channel response")))?;
        serde_json::from_value(value).map_err(|e| Error::Serde(e.to_string()))
    };
    let writer = end("writer")?;
    let reader = end("reader")?;
    Ok((writer, reader))
}

/// Mint a fresh router-owned iii channel for one provider attempt.
pub async fn create_router_channel(iii: &IIIClient) -> Result<RouterChannel, Error> {
    let (writer_ref, reader_ref) = create_pipe_refs(iii).await?;
    let input = InputStream::from_ref(reader_ref, iii)?;

    // reader: a pump task forwards the text frames into an mpsc the relay
    // can select on; end of stream or an error drops the sender (EOF).
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let cancel = Arc::new(Notify::new());
    let cancel_pump = cancel.clone();
    tokio::spawn(async move {
        let mut frames = input.frames();
        loop {
            tokio::select! {
                frame = frames.next() => match frame {
                    Some(Ok(ChannelItem::Text(text))) => {
                        if tx.send(text).is_err() {
                            break;
                        }
                    }
                    Some(Ok(ChannelItem::Binary(_))) => {}
                    _ => break, // closed or errored -> sender drops -> EOF
                },
                _ = cancel_pump.notified() => {
                    input.cancel().await;
                    break;
                }
            }
        }
    });

    Ok(RouterChannel {
        writer: spawn_writer_forwarder(iii.clone(), writer_ref.clone()),
        writer_ref,
        reader: Box::new(SdkReader {
            rx,
            close_flag: Arc::new(AtomicBool::new(false)),
            cancel,
        }),
    })
}

/// Build a sink for a caller-supplied writer_ref; same forwarder-task bridge.
pub async fn open_sink(iii: &IIIClient, r: &StreamChannelRef) -> Result<Arc<dyn FrameSink>, Error> {
    Ok(spawn_writer_forwarder(iii.clone(), r.clone()))
}
