// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Library facade for `sandbox-libkrun`, the libkrun sandbox adapter.
//!
//! Exposes the VM substrate so integration tests exercise the real code
//! instead of duplicate copies.

// Private offline generation primitives. No runtime issuer or remote entrypoint.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
mod compat_generation;

pub mod cli;
pub(crate) mod core;
pub mod daemon_exit;
pub mod sandbox_daemon;
pub mod wire;
pub mod worker_vm;
pub(crate) mod worker_admission;

pub use cli::app::{Cli, Commands};
pub use cli::vm_boot::VmBootArgs;
