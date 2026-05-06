//! Shared helpers for the `shell-*` worker family.
//!
//! - `channels` drains/fills `StreamChannelRef`s.
//! - `session` resolves `sandbox_id` from args or session state.
//! - `errors` maps `IIIError` and missing-sandbox cases into `ToolResult`.

pub mod channels;
pub mod errors;
pub mod session;

pub use channels::{drain_ref, fill_ref};
pub use errors::ShellError;
pub use session::{
    load_sandbox_id_from_state, parse_sandbox_id_from_args, parse_session_id_from_args,
    resolve_sandbox_id,
};
