// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Library facade for `sandbox-builtin`, the builtin sandbox adapter.
//!
//! Exposes the VM substrate so integration tests exercise the real code
//! instead of duplicate copies.

pub mod cli;
pub(crate) mod core;
pub mod daemon_exit;
pub mod sandbox_daemon;
pub mod wire;

pub use cli::app::{Cli, Commands};
pub use cli::vm_boot::VmBootArgs;
