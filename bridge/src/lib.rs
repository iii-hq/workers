//! Bridge worker: connects the local engine to a remote iii instance.
//! Registers NO trigger type — only functions (`bridge.invoke`,
//! `bridge.invoke_async`, plus configured forward/expose entries).

pub mod boot;
pub mod config;
pub mod configuration;
pub mod functions;
pub mod manifest;
