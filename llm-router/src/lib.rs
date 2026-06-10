//! llm-router: routing, provider registry, credentials, model catalog, and the
//! failure contract — spec: tech-specs/2026-06-agentic/llm-router.md.

pub mod bus;
pub mod catalog;
pub mod chat;
pub mod config;
pub mod registry;
pub mod routing;
pub mod settings;
pub mod testkit;
pub mod triggers;
pub mod types;
