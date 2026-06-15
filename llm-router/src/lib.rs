//! llm-router: routing, provider registry, credentials, model catalog, and the
//! failure contract — spec: tech-specs/2026-06-agentic/llm-router.md.

pub mod catalog;
pub mod channels;
pub mod chat;
pub mod config;
pub mod manifest;
pub mod register;
pub mod registry;
pub mod routing;
pub mod settings;
pub mod state;
pub mod testkit;
pub mod triggers;
pub mod types;
