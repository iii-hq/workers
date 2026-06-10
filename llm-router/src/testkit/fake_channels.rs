//! Engine-faithful fake of iii streaming channels (semantics copied from
//! engine/src/workers/worker/channels.rs): single reader, writes fail after
//! the reader closes, reader sees EOF after the writer closes, in-order
//! delivery.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::types::channel::{ChannelDirection, StreamChannelRef};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::bus::BusError;
use crate::chat::relay::{
    CallerGone, ChannelFactory, FrameSink, ReadEvent, RelayRead, RouterChannel,
};

#[derive(Default)]
struct Inner {
    queue: Mutex<VecDeque<String>>,
    flags: Mutex<Flags>,
    notify: Notify,
}
#[derive(Default)]
struct Flags {
    writer_closed: bool,
    reader_closed: bool,
}

#[derive(Clone)]
pub struct FakeWriter {
    inner: Arc<Inner>,
}

impl FrameSink for FakeWriter {
    fn send(&self, msg: &str) -> Result<(), CallerGone> {
        let flags = self.inner.flags.lock().unwrap();
        if flags.reader_closed || flags.writer_closed {
            return Err(CallerGone); // engine: send fails once the receiver dropped
        }
        drop(flags);
        self.inner.queue.lock().unwrap().push_back(msg.to_string());
        self.inner.notify.notify_waiters();
        Ok(())
    }
    fn close(&self) {
        self.inner.flags.lock().unwrap().writer_closed = true;
        self.inner.notify.notify_waiters();
    }
    fn is_closed(&self) -> bool {
        let flags = self.inner.flags.lock().unwrap();
        flags.reader_closed || flags.writer_closed
    }
}

pub struct FakeReader {
    inner: Arc<Inner>,
}

#[async_trait::async_trait]
impl RelayRead for FakeReader {
    async fn next(&mut self, timeout: Duration) -> ReadEvent {
        loop {
            if let Some(msg) = self.inner.queue.lock().unwrap().pop_front() {
                return ReadEvent::Msg(msg);
            }
            {
                let flags = self.inner.flags.lock().unwrap();
                if flags.writer_closed || flags.reader_closed {
                    return ReadEvent::Eof;
                }
            }
            tokio::select! {
                _ = self.inner.notify.notified() => {}
                _ = tokio::time::sleep(timeout) => return ReadEvent::Timeout,
            }
        }
    }
    fn closer(&self) -> Arc<dyn Fn() + Send + Sync> {
        let inner = self.inner.clone();
        Arc::new(move || {
            inner.flags.lock().unwrap().reader_closed = true;
            inner.notify.notify_waiters();
        })
    }
    fn close(&self) {
        (self.closer())();
    }
}

pub struct FakeChannel {
    pub channel_id: String,
    pub writer_ref: StreamChannelRef,
    pub reader_ref: StreamChannelRef,
    pub writer: FakeWriter,
    pub reader: FakeReader,
}

impl FakeChannel {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let inner = Arc::new(Inner::default());
        let channel_id = format!("ch_{}", Uuid::new_v4());
        let access_key = Uuid::new_v4().to_string();
        FakeChannel {
            writer_ref: StreamChannelRef {
                channel_id: channel_id.clone(),
                access_key: access_key.clone(),
                direction: ChannelDirection::Write,
            },
            reader_ref: StreamChannelRef {
                channel_id: channel_id.clone(),
                access_key,
                direction: ChannelDirection::Read,
            },
            channel_id,
            writer: FakeWriter {
                inner: inner.clone(),
            },
            reader: FakeReader { inner },
        }
    }
}

/// Registry of live fake channels: a scripted provider receives a forwarded
/// writer_ref and looks up the live writer here (no hydration in fakes).
#[derive(Default)]
pub struct ChannelHub {
    writers: Mutex<HashMap<String, FakeWriter>>,
}

impl ChannelHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn writer_for(&self, r: &StreamChannelRef) -> Option<FakeWriter> {
        self.writers.lock().unwrap().get(&r.channel_id).cloned()
    }
}

#[async_trait::async_trait]
impl ChannelFactory for ChannelHub {
    async fn create(&self) -> Result<RouterChannel, BusError> {
        let ch = FakeChannel::new();
        self.writers
            .lock()
            .unwrap()
            .insert(ch.channel_id.clone(), ch.writer.clone());
        Ok(RouterChannel {
            writer_ref: ch.writer_ref,
            reader: Box::new(ch.reader),
            writer: Arc::new(ch.writer),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::relay::{FrameSink, ReadEvent, RelayRead};
    use std::time::Duration;

    #[tokio::test]
    async fn delivers_messages_in_order_then_eof_after_writer_close() {
        let ch = FakeChannel::new();
        ch.writer.send("a").unwrap();
        ch.writer.send("b").unwrap();
        ch.writer.close();
        let mut reader = ch.reader;
        assert!(
            matches!(reader.next(Duration::from_millis(100)).await, ReadEvent::Msg(m) if m == "a")
        );
        assert!(
            matches!(reader.next(Duration::from_millis(100)).await, ReadEvent::Msg(m) if m == "b")
        );
        assert!(matches!(
            reader.next(Duration::from_millis(100)).await,
            ReadEvent::Eof
        ));
    }

    #[tokio::test]
    async fn writes_fail_after_reader_closes() {
        let ch = FakeChannel::new();
        ch.writer.send("a").unwrap();
        ch.reader.close();
        assert!(ch.writer.send("b").is_err());
        assert!(ch.writer.is_closed());
    }

    #[tokio::test]
    async fn read_times_out_when_no_message_arrives() {
        let ch = FakeChannel::new();
        let mut reader = ch.reader;
        assert!(matches!(
            reader.next(Duration::from_millis(20)).await,
            ReadEvent::Timeout
        ));
    }

    #[tokio::test]
    async fn pending_read_resolves_when_a_message_arrives_later() {
        let ch = FakeChannel::new();
        let writer = ch.writer.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            writer.send("late").unwrap();
        });
        let mut reader = ch.reader;
        assert!(
            matches!(reader.next(Duration::from_millis(1000)).await, ReadEvent::Msg(m) if m == "late")
        );
    }

    #[tokio::test]
    async fn hub_resolves_writers_from_forwarded_refs_and_acts_as_factory() {
        use crate::chat::relay::ChannelFactory;
        let hub = ChannelHub::new();
        let router_ch = hub.create().await.unwrap();
        let writer = hub.writer_for(&router_ch.writer_ref).unwrap();
        writer.send("via-ref").unwrap();
        let mut reader = router_ch.reader;
        assert!(
            matches!(reader.next(Duration::from_millis(100)).await, ReadEvent::Msg(m) if m == "via-ref")
        );
    }
}
