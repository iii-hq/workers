// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "sandbox-builtin",
    bin_name = "sandbox-builtin",
    version,
    about = "Builtin sandbox adapter"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Internal: sandbox RPC daemon. Started automatically by the iii
    /// engine when `iii-sandbox` appears in config.yaml.
    #[command(name = "sandbox-daemon", hide = true)]
    SandboxDaemon(SandboxDaemonArgs),

    /// Internal: boot a libkrun VM (crash-isolated subprocess)
    #[command(name = "__vm-boot", hide = true)]
    VmBoot(Box<super::vm_boot::VmBootArgs>),
}

/// Arguments for the `sandbox-daemon` subcommand. Started by the iii
/// engine as a child process; rarely invoked directly.
#[derive(Args, Debug)]
pub struct SandboxDaemonArgs {
    /// Path to the sandbox daemon's YAML config (flat `SandboxConfig` shape).
    #[arg(long, default_value = "./config.yaml")]
    pub config: String,

    /// Engine WebSocket URL to connect back to.
    #[arg(long, env = "III_ENGINE_URL", default_value = "ws://127.0.0.1:49134")]
    pub engine: String,
}

#[cfg(test)]
mod daemon_engine_arg_tests {
    use super::*;
    use clap::Parser;

    fn sandbox_engine(argv: &[&str]) -> String {
        match Cli::try_parse_from(argv).expect("should parse").command {
            Commands::SandboxDaemon(args) => args.engine,
            _ => panic!("expected SandboxDaemon variant"),
        }
    }

    /// The engine exports III_ENGINE_URL when spawning the sandbox daemon
    /// (engine/src/workers/external.rs); it must honor it so
    /// they connect back to the spawning engine's actual worker-listener
    /// port, not the hardcoded default (MOT-3970). Explicit --engine
    /// still wins (clap precedence: flag > env > default).
    #[test]
    #[serial_test::serial]
    fn daemon_engine_args_honor_env_and_flag_precedence() {
        // SAFETY: edition 2024 requires unsafe wrap; test is #[serial].
        unsafe {
            std::env::set_var("III_ENGINE_URL", "ws://127.0.0.1:55555");
        }
        // Capture first, assert after cleanup: a failing assert must not
        // leak the var into later env-reading tests in this process.
        let sandbox_env = sandbox_engine(&["sandbox-builtin", "sandbox-daemon"]);
        let sandbox_flag = sandbox_engine(&[
            "sandbox-builtin",
            "sandbox-daemon",
            "--engine",
            "ws://flag:1",
        ]);
        // SAFETY: edition 2024 requires unsafe wrap; test is #[serial].
        unsafe {
            std::env::remove_var("III_ENGINE_URL");
        }
        let default = sandbox_engine(&["sandbox-builtin", "sandbox-daemon"]);

        assert_eq!(sandbox_env, "ws://127.0.0.1:55555");
        assert_eq!(
            sandbox_flag, "ws://flag:1",
            "explicit --engine must beat env"
        );
        assert_eq!(default, "ws://127.0.0.1:49134", "no env, no flag: default");
    }
}
