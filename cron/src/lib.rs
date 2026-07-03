/// The trigger type this worker registers. Always `cron` -- the built-in
/// `iii-cron` worker must be removed from the engine config before this
/// worker can boot (see [`boot::start`]'s guard).
pub const TRIGGER_TYPE: &str = "cron";

pub mod boot;
pub mod config;
pub mod configuration;
pub mod locks;
pub mod manifest;
pub mod scheduler;
pub mod trigger;
