//! Event sink trait used by the orchestrator and the TUI.
//!
//! `EventSink` is the abstract event consumer used by `harness-tui`'s
//! `ChannelSink` (`workers/harness-tui/src/sink.rs`). Production event
//! emission lives in `turn-orchestrator::events` (see
//! `workers/primitives/turn-orchestrator/src/events.rs`).

use async_trait::async_trait;
use harness_types::AgentEvent;

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}
