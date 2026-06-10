//! Relay seams over iii streaming channels. The router never hands a provider
//! the caller's channel: per attempt it mints a fresh iii channel (via
//! `ChannelFactory`), reads provider frames itself (`RelayRead`), and forwards
//! each frame to the caller's writer (`FrameSink`).
use std::sync::Arc;
use std::time::Duration;

use crate::types::channel::StreamChannelRef;

use crate::bus::BusError;

#[derive(Debug)]
pub enum ReadEvent {
    Msg(String),
    Eof,
    Timeout,
}

/// What the relay loop reads. Implemented by the fake channel (testkit) and
/// the real SDK adapter (channels.rs, Task 18).
#[async_trait::async_trait]
pub trait RelayRead: Send {
    async fn next(&mut self, timeout: Duration) -> ReadEvent;
    /// Shared close handle — router::abort closes the provider channel from
    /// another task while the relay loop is blocked in next().
    fn closer(&self) -> Arc<dyn Fn() + Send + Sync>;
    fn close(&self);
}

#[derive(Debug, thiserror::Error)]
#[error("caller channel closed")]
pub struct CallerGone;

/// What the relay loop writes (the caller's channel, or an internal one for
/// router::complete). send() fails once the peer reader dropped.
pub trait FrameSink: Send + Sync {
    fn send(&self, msg: &str) -> Result<(), CallerGone>;
    fn close(&self);
    fn is_closed(&self) -> bool;
}

/// A router-owned channel, fresh per provider attempt (relay topology).
pub struct RouterChannel {
    pub writer_ref: StreamChannelRef,
    pub reader: Box<dyn RelayRead>,
    pub writer: Arc<dyn FrameSink>, // used by router::complete's internal channel
}

#[async_trait::async_trait]
pub trait ChannelFactory: Send + Sync {
    async fn create(&self) -> Result<RouterChannel, BusError>;
}
