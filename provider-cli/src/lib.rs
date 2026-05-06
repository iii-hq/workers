//! `provider::cli::*` — wrap installed CLIs (claude, codex, opencode,
//! openclaw, hermes, pi, gemini, cursor-agent) as a provider. Calls
//! `shell::bash::which` to probe and `shell::bash::exec` to drive each CLI.
//! Graduated from roster/workers/provider-cli (TS) in P4.

pub mod register;
pub mod shapes;

pub use register::{register_with_iii, ProviderCliFunctionRefs};
pub use shapes::{CliShape, CLI_SHAPES};
