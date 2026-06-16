//! `session-manager` — durable, reactive store for conversations.
//!
//! A session is an append-only log of typed message entries (with
//! optional fork branches) plus a small metadata record (`title`,
//! `description`, coarse `status`, app-defined `metadata`). The worker
//! is a **pure storage + notification surface**: it binds no triggers of
//! its own and runs no agent logic.
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
//! - [`events`] — the six custom trigger types this worker registers
//!   (`session::created`, `session::message-added`,
//!   `session::message-updated`, `session::status-changed`,
//!   `session::meta-updated`, `session::deleted`), the per-binding
//!   config filters, the `Emitter` fan-out, and the cross-instance
//!   propagation plumbing (`EventEnvelope`, `RemotePublisher`,
//!   `attach_bridge_relay`): the main instance is the single fan-out
//!   point and every bridged instance re-emits its feed locally.
//! - [`functions`] — the 14 `session::*` typed function handlers plus
//!   the internal `session::store::*` protocol.
//! - [`runtime`] — the hot-swappable storage + event runtime built from
//!   the `adapter` config, so an adapter change reloads live (no restart).
//! - [`resync`] — replays the new store's state through the trigger
//!   fan-out after an adapter swap so subscribers stay real-time.

pub mod config;
pub mod configuration;
pub mod error;
pub mod events;
pub mod functions;
pub mod manifest;
pub mod resync;
pub mod runtime;
pub mod service;
pub mod store;
pub mod types;
