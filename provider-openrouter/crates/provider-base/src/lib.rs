//! Shared infrastructure for harness provider crates.
//!
//! - Surrogate sanitisation: strip unpaired UTF-16 surrogates from outgoing
//!   text so JSON serialisation does not fail on emoji-heavy input.
//! - HTTP client construction with sane defaults (120s timeout, gzip).
//! - SSE event chunking: a small buffered reader that yields complete `data: …`
//!   blocks separated by blank lines.
//! - Error classification: thin wrapper over `overflow-classify` that returns
//!   the harness `ErrorKind` from an HTTP status + body text.
//! - Generic OpenAI-compatible Chat Completions streaming: reused by every
//!   provider that speaks the OpenAI completions wire shape (Groq, Cerebras,
//!   xAI, OpenRouter, DeepSeek, Mistral, Fireworks, Kimi, MiniMax, z.ai,
//!   HuggingFace, Vercel AI Gateway, OpenCode Zen, OpenCode Go).

pub mod auth;
pub mod errors;
pub mod iii_register;
pub mod openai_compat;
pub mod sse;

pub use auth::{credential_to_string, fetch_credential};
pub use errors::{classify_provider_error, error_event};
pub use iii_register::register_provider_complete;
pub use openai_compat::{stream_chat_completions, ChatCompletionsConfig, OpenAICompatRequest};
pub use sse::{parse_sse_block, sanitize_surrogates};
