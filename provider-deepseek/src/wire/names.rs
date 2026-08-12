//! iii function ids ↔ DeepSeek tool names. Upstream enforces
//! `^[a-zA-Z0-9_-]{1,64}$`; bus ids use `::` separators. Shared codec
//! (and its tests) live in `llm_router::provider_scaffold::names`.

pub use llm_router::provider_scaffold::names::{decode_tool_name, encode_tool_name};
