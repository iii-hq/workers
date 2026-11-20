//! iii function ids ↔ Gemini tool names. Upstream accepts a name that starts
//! with a letter or underscore, then `[A-Za-z0-9_.:-]`, at most 128
//! characters; bus ids use `::` separators. Shared codec (and its tests) live
//! in `llm_router::provider_scaffold::names`.

pub use llm_router::provider_scaffold::names::{decode_tool_name, encode_tool_name};
