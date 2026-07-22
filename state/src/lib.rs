/// The trigger type this worker registers. Always `state` -- the built-in
/// `iii-state` worker must be removed from the engine config before this
/// worker can boot (see [`boot::start`]'s guard).
pub const TRIGGER_TYPE: &str = "state";

pub mod adapters;
pub mod boot;
pub mod condition;
pub mod config;
pub mod configuration;
pub mod events;
pub mod functions;
pub mod manifest;
pub mod store;
pub mod structs;
pub mod trigger;
pub mod ui;
pub mod update_ops;
