//! `shell-filesystem` worker — registers `shell::filesystem::<op>` functions
//! for the turn orchestrator and agent tooling.

pub mod ops;
pub mod register;

pub use register::register_with_config;
pub use register::register_with_iii;
