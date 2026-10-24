// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

use clap::Parser;
use sandbox_builtin::{Cli, Commands};

/// A `stderr` tracing writer that never reports I/O errors back to the fmt
/// layer. The fmt layer's sole error path is an `eprintln!`, which PANICS
/// when stderr is *also* a broken pipe: Compose owns the adapter's stdout and
/// stderr pipes, so once Compose is gone the next log line would panic the
/// adapter (exit 101) before it stops its VMs. Swallowing the write error
/// keeps it running until its shutdown signal; for a hand run it turns a
/// `… | head` EPIPE into a silent drop instead of a panic.
struct ResilientStderr;

impl std::io::Write for ResilientStderr {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::Write::write_all(&mut std::io::stderr(), buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::Write::flush(&mut std::io::stderr());
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for ResilientStderr {
    type Writer = ResilientStderr;
    fn make_writer(&self) -> Self::Writer {
        ResilientStderr
    }
}

fn main() -> anyhow::Result<()> {
    // FIRST, before the tokio runtime spawns worker threads: capture (and
    // scrub from the env) any inherited lifeline facts. The capture mutates
    // the process environment, which is only sound while single-threaded —
    // see daemon_exit::capture_early.
    sandbox_builtin::daemon_exit::capture_early();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(ResilientStderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli_args = Cli::parse();

    let exit_code = match cli_args.command {
        None => match sandbox_builtin::sandbox_daemon::serve().await {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!(error = %e, "sandbox-builtin exited with error");
                1
            }
        },
        Some(Commands::VmBoot(args)) => {
            // Run the VM on a dedicated OS thread: `msb_krun`'s virtio-blk
            // Drop impls call `tokio::Runtime::block_on` for async shutdown,
            // which panics inside our `#[tokio::main]` runtime. The std
            // thread gives those drops a runtime-free context.
            //
            // `vm_boot::run` is `-> !`; a clean join is a bug, exit 1.
            //
            // TODO(msb_krun upstream): remove this std::thread dispatch
            // once virtio-blk Drop uses `Handle::try_current()` instead
            // of unconditional `block_on`.
            let handle = std::thread::Builder::new()
                .name("sandbox-builtin-vm-boot".to_string())
                .spawn(move || sandbox_builtin::cli::vm_boot::run(&args))
                .expect("failed to spawn vm-boot thread");
            match handle.join() {
                Err(_) => {
                    eprintln!("error: vm-boot thread panicked");
                    std::process::exit(1);
                }
                Ok(_never) => {
                    eprintln!("error: vm-boot returned without std::process::exit");
                    std::process::exit(1);
                }
            }
        }
    };

    std::process::exit(exit_code);
}
