//! LLM tool catalog. After the iii-native refactor (spec
//! `docs/superpowers/specs/2026-05-07-iii-native-harness-design.md`) the
//! catalog has exactly one entry, `agent_call`. The schema lives in
//! `agent_call.rs`; this module re-exports it so existing callers keep
//! compiling. Delete this file once those callers are migrated to import
//! `crate::agent_call::agent_call_tool` directly.

pub use crate::agent_call::agent_call_tool;
