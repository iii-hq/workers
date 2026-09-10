//! Shared backend selection used by every `shell::fs::*` handler.

use std::sync::Arc;

use uuid::Uuid;

use crate::fs::sandbox::{IiiTriggerFwd, SandboxFsBackend};
use crate::fs::{FsBackend, Target};

pub fn pick_backend(
    target: Target,
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::IIIClient,
    sandbox_enabled: bool,
) -> Arc<dyn FsBackend> {
    match target {
        Target::Host => host,
        Target::Sandbox { sandbox_id } => sandbox_for(sandbox_id, iii, sandbox_enabled),
    }
}

fn sandbox_for(id: Uuid, iii: iii_sdk::IIIClient, enabled: bool) -> Arc<dyn FsBackend> {
    Arc::new(SandboxFsBackend::new(
        Arc::new(IiiTriggerFwd::new(iii)),
        enabled,
        id,
    ))
}
