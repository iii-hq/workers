/// The trigger type this worker registers. Always `durable:subscriber` --
/// the built-in `iii-queue` worker must be removed from the engine config
/// before this worker can boot (see [`boot::start`]'s guard).
pub const TRIGGER_TYPE: &str = "durable:subscriber";

pub mod adapter;
pub mod adapters;
pub mod boot;
pub mod config;
pub mod configuration;
pub mod function_queues;
pub mod functions;
pub mod manifest;
pub mod store;
pub mod subscriber_config;
pub mod trigger;
