// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

use clap::{Parser, Subcommand};

/// With no subcommand, serves the adapter.
#[derive(Parser, Debug)]
#[command(
    name = "sandbox-builtin",
    bin_name = "sandbox-builtin",
    version,
    about = "Builtin sandbox adapter"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Internal: boot a libkrun VM (crash-isolated subprocess)
    #[command(name = "__vm-boot", hide = true)]
    VmBoot(Box<super::vm_boot::VmBootArgs>),
}
