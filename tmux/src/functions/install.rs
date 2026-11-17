//! `tmux::install` and `tmux::install::status` (simple-installs D7): the first
//! starts or joins the background install and answers at once, the second is
//! what a caller polls.

use std::sync::Arc;

use iii_sdk::IIIClient;

use super::{Ctx, EmptyInput, FunctionSpec, register_fn, spec};
use crate::install::{InstallStatus, Installer};

pub const INSTALL: &str = "tmux::install";
pub const INSTALL_STATUS: &str = "tmux::install::status";

const INSTALL_DESC: &str = "Install the tmux program on this computer, in the background, and answer at once: the system's package manager when this user may run it without a password (Homebrew, MacPorts, apt, dnf, pacman, zypper, apk), otherwise tmux's official build into ~/.local/bin. Starting during a run joins it. Poll tmux::install::status.";
const INSTALL_STATUS_DESC: &str = "Where tmux::install is: idle (nothing ran since the worker started), running with its step, label and hints, succeeded with the path and version, or failed with one sentence and the log.";

struct InstallCtx {
    ctx: Arc<Ctx>,
    installer: Installer,
}

async fn install(ctx: &InstallCtx, _input: EmptyInput) -> Result<InstallStatus, String> {
    let monitor = ctx.ctx.monitor.clone();
    Ok(ctx
        .installer
        .start(ctx.ctx.collector.clone(), Arc::new(move || monitor.poke())))
}

async fn status(ctx: &InstallCtx, _input: EmptyInput) -> Result<InstallStatus, String> {
    Ok(ctx.installer.status())
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, InstallStatus>(INSTALL, INSTALL_DESC),
        spec::<EmptyInput, InstallStatus>(INSTALL_STATUS, INSTALL_STATUS_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    let ctx = Arc::new(InstallCtx {
        ctx: ctx.clone(),
        installer: Installer::default(),
    });
    register_fn!(iii, ctx, INSTALL, INSTALL_DESC, EmptyInput, install);
    register_fn!(
        iii,
        ctx,
        INSTALL_STATUS,
        INSTALL_STATUS_DESC,
        EmptyInput,
        status
    );
}
