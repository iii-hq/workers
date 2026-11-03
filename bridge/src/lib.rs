//! Bridge worker: connects the local engine to a remote iii instance.
//! Provides no event functions — only callable functions (`bridge.invoke`,
//! `bridge.invoke_async`, plus configured forward/expose entries).

pub mod boot;
pub mod config;
pub mod configuration;
pub mod functions;
pub mod manifest;
