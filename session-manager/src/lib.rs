//! `session-manager` — durable, reactive store for conversations.
//!
//! A session is an append-only log of typed message entries (with
//! optional fork branches) plus a small metadata record (`title`,
//! `description`, coarse `status`, app-defined `metadata`). The worker
//! is a **pure storage + notification surface**: it runs no agent logic,
//! and observes only its own configuration entry and, as a bridged
//! instance, the main instance's event feed.
//!
//! Layering (each layer is independently testable):
//!
//! - [`types`] — the cross-cutting wire contracts (`AgentMessage`,
//!   `ContentBlock`, `SessionEntry`, `SessionMeta`).
//! - [`store`] — the `SessionStore` trait with an `FsStore` (default:
//!   one append-only JSONL file per session) and a `BridgeStore`
//!   (defers raw storage to a main session-manager on another iii
//!   instance via the internal `session::store::*` protocol).
//! - [`service`] — all domain logic; returns the response **and** the
//!   events to emit, so emission stays at the edge.
//! - [`events`] — the six private event functions this worker hosts
//!   (`session::on-created`, `session::on-message-added`,
//!   `session::on-message-updated`, `session::on-status-changed`,
//!   `session::on-meta-updated`, `session::on-deleted`), the `Emitter`
//!   that calls them while they are observed, and the cross-instance
//!   propagation plumbing (`EventEnvelope`, `session::store::on-events`,
//!   `RemotePublisher`, `attach_bridge_relay`): the main instance is the
//!   single fan-out point and every bridged instance re-emits its feed
//!   locally.
//! - [`pagination`] — block segmentation and elision behind the lazy
//!   transcript readers (`session::messages-tail` / `messages-range`).
//! - [`functions`] — the 21 `session::*` typed function handlers plus
//!   the internal `session::store::*` protocol.
//! - [`runtime`] — the hot-swappable storage + event runtime built from
//!   the `adapter` config, so an adapter change reloads live (no restart).
//! - [`resync`] — replays the new store's state through the event
//!   functions after an adapter swap so observers stay real-time.

pub mod config;
pub mod configuration;
pub mod error;
pub mod events;
pub mod functions;
pub mod manifest;
pub mod pagination;
pub mod resync;
pub mod runtime;
pub mod service;
pub mod store;
pub mod surface;
pub mod types;
