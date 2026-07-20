//! iii function ids ↔ OpenAI-style tool function names. `::` is not a valid
//! character in most chat-template tool-name grammars, so bus ids get
//! sanitized to `__` on the wire. Shared codec (and its tests) live in
//! `llm_router::provider_scaffold::names`.

pub use llm_router::provider_scaffold::names::{decode_tool_name, encode_tool_name};
