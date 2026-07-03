//! Exec backend hierarchy. Mirrors `fs::{FsBackend, host, sandbox}` for
//! `shell::exec` / `shell::exec_bg`.

pub mod backend;
pub mod error;
pub mod host;
pub mod policy;
pub mod sandbox;

pub use backend::{ExecBackend, ExecCallResult, ExecOutcome};
// Per-call cwd/env gating surface. `ExecOverrides` is the validated bundle
// threaded into `build_command`; `build_overrides` does the gating. Exposed
// for the handlers and (lib consumers) the test harness.
#[allow(unused_imports)]
pub use policy::{build_overrides, ExecOverrides};
// These two re-exports are the public-API surface for downstream
// consumers of `shell::exec::*` and the stable surface for
// internal `crate::exec::{parse_argv, build_command, run_to_completion,
// HostExecBackend, ExecError}` import paths. Rustc flags them as
// unused inside the binary target (the binary uses fully-qualified
// `crate::exec::host::...` paths in its own module tree); the lib
// target and external consumers DO use them. Suppressed deliberately.
#[allow(unused_imports)]
pub use error::ExecError;
#[allow(unused_imports)]
pub use host::{build_command, parse_argv, run_to_completion, HostExecBackend};
