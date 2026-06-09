//! Backend selection for `shell::exec` / `shell::exec_bg`. Mirrors
//! `functions::fs_dispatch::pick_backend`.

use std::sync::Arc;

use uuid::Uuid;

use crate::config::ShellConfig;
use crate::exec::host::HostExecBackend;
use crate::exec::sandbox::SandboxExecBackend;
use crate::exec::ExecBackend;
use crate::target::Target;
use crate::triggers::IiiTriggerFwd;

pub fn pick_exec_backend(
    target: Target,
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::III,
) -> Arc<dyn ExecBackend> {
    match target {
        Target::Host => Arc::new(HostExecBackend::new(cfg)),
        Target::Sandbox { sandbox_id } => sandbox_for(sandbox_id, iii, cfg.sandbox.enabled),
    }
}

fn sandbox_for(id: Uuid, iii: iii_sdk::III, enabled: bool) -> Arc<dyn ExecBackend> {
    Arc::new(SandboxExecBackend::new(
        Arc::new(IiiTriggerFwd::new(iii)),
        enabled,
        id,
    ))
}

// No unit tests in this module: `pick_exec_backend`'s match arms are trivial
// constructors that would require a real `iii_sdk::III` to exercise. The
// ExecError -> wire-code conversion is covered by
// `exec::error::tests` (the `From<ExecError> for IIIError` Remote lift).
