//! RabbitMQ transport adapter.
//!
//! Ported from the engine builtin `iii-queue` worker's RabbitMQ adapter
//! (`engine/src/workers/queue/adapters/rabbitmq/`, all 8 modules). Gated
//! behind the `rabbitmq` cargo feature (on by default) — the whole module
//! tree, and every individual file (`#![cfg(feature = "rabbitmq")]`), compile
//! out entirely with `--no-default-features`.

pub mod naming;
pub mod types;
