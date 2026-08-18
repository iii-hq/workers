//! Real iii channel plumbing for the relay (no abstraction — these are the
//! concrete bridges over `iii_sdk` channels).
//! `create_router_channel` mints a router-owned iii channel through the
//! engine's `default` namespace; `open_sink` builds a `ChannelWriter` directly
//! from a forwarded `writer_ref` (the Rust SDK does not hydrate refs in
//! payloads).
//!
//! FrameSink::send is sync, bridging to the async ChannelWriter via an
//! unbounded mpsc + forwarder task — a send failure is observed one frame
//! late, same as the WebSocket behavior in the Node SDK. The SDK reader only
//! dispatches text frames while `next_binary()` is polled, so a pump task
//! drives it; pump exit (close/error) drops the sender → EOF for the relay.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::channel::{Channel, ChannelReader, ChannelWriter, StreamChannelRef};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{errors::Error, IIIClient};
use serde_json::json;
use tokio::sync::{mpsc, Notify};

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

/// Spawn the forwarder task that owns the async ChannelWriter.
fn spawn_writer_forwarder(writer: ChannelWriter) -> Arc<SdkSink> {
    let (tx, mut rx) = mpsc::unbounded_channel::<SinkMsg>();
    let closed = Arc::new(AtomicBool::new(false));
    let closed_for_task = closed.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                SinkMsg::Frame(m) => {
                    if writer.send_message(&m).await.is_err() {
                        closed_for_task.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                SinkMsg::Close => break,
            }
        }
        let _ = writer.close().await;
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

/// Mint a fresh router-owned iii channel for one provider attempt.
pub async fn create_router_channel(iii: &IIIClient) -> Result<RouterChannel, Error> {
    // SDK 0.23 makes an unqualified trigger inherit this worker's project
    // namespace. Channel creation is a static engine function, so target the
    // engine namespace explicitly while provider calls continue to inherit the
    // project namespace elsewhere.
    let result = iii
        .trigger(
            TriggerRequest {
                function_id: "engine::channels::create".to_string(),
                payload: json!({ "buffer_size": null }),
                action: None,
                timeout_ms: None,
            }
            .namespace("default"),
        )
        .await?;
    let writer_ref = channel_ref(&result, "writer")?;
    let reader_ref = channel_ref(&result, "reader")?;
    let channel = Channel {
        writer: ChannelWriter::new(iii.address(), &writer_ref),
        reader: ChannelReader::new(iii.address(), &reader_ref),
        writer_ref,
        reader_ref,
    };

    // reader: bridge on_message + a pump task into an mpsc the relay can
    // select on. The pump drives next_binary(), which is what dispatches
    // text frames to the callback; Ok(None)/Err = channel closed.
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let cancel = Arc::new(Notify::new());
    {
        let cb_tx = tx.clone();
        channel
            .reader
            .on_message(move |msg| {
                let _ = cb_tx.send(msg);
            })
            .await;
    }
    drop(tx); // the callback inside the reader holds the only live sender
    let reader_pump = channel.reader;
    let cancel_pump = cancel.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                r = reader_pump.next_binary() => {
                    if !matches!(r, Ok(Some(_))) {
                        break; // closed or errored → reader (and sender) drop → EOF
                    }
                }
                _ = cancel_pump.notified() => {
                    let _ = reader_pump.close().await;
                    break;
                }
            }
        }
    });

    Ok(RouterChannel {
        writer_ref: channel.writer_ref,
        reader: Box::new(SdkReader {
            rx,
            close_flag: Arc::new(AtomicBool::new(false)),
            cancel,
        }),
        writer: spawn_writer_forwarder(channel.writer),
    })
}

fn channel_ref(result: &serde_json::Value, field: &str) -> Result<StreamChannelRef, Error> {
    serde_json::from_value(
        result
            .get(field)
            .cloned()
            .ok_or_else(|| Error::Serde(format!("missing '{field}' in channel response")))?,
    )
    .map_err(Error::from)
}

/// Build a sink for a caller-supplied writer_ref; same forwarder-task bridge.
pub async fn open_sink(iii: &IIIClient, r: &StreamChannelRef) -> Result<Arc<dyn FrameSink>, Error> {
    let writer = ChannelWriter::new(iii.address(), r);
    Ok(spawn_writer_forwarder(writer))
}
