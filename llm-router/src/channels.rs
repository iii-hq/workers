//! Real iii channel plumbing for the relay (no abstraction — these are the
//! concrete bridges over `iii_sdk` channels).
//! `create_router_channel` mints a router-owned iii channel via
//! `iii_sdk::helpers::create_channel`; `open_sink` builds a `ChannelWriter`
//! directly from a forwarded `writer_ref` (the Rust SDK does not hydrate refs
//! in payloads).
//!
//! FrameSink::send is sync, bridging to the async ChannelWriter via an
//! unbounded mpsc + forwarder task — a send failure is observed one frame
//! late, same as the WebSocket behavior in the Node SDK. The SDK reader only
//! dispatches text frames while `next_binary()` is polled, so a pump task
//! drives it; pump exit (close/error) drops the sender → EOF for the relay.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::channel::{ChannelWriter, StreamChannelRef};
use iii_sdk::{errors::Error, IIIClient};
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
    let channel = iii_sdk::helpers::create_channel(iii, None).await?;

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

/// Build a sink for a caller-supplied writer_ref; same forwarder-task bridge.
pub async fn open_sink(iii: &IIIClient, r: &StreamChannelRef) -> Result<Arc<dyn FrameSink>, Error> {
    let writer = ChannelWriter::new(iii.address(), r);
    Ok(spawn_writer_forwarder(writer))
}
