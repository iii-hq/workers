//! Kits: bundles of agent profiles, skills and registry workers (by semver
//! range) published to the registry as `<author-handle>/<kit-name>`.
//!
//! * [`paths`]    — where `kits.lock` and pending plans live.
//! * [`lock`]     — `kits.lock`: installed version, worker ranges, and the
//!   base hash of every installed file.
//! * [`kitref`]   — `<handle>/<name>[@version|tag|range]`.
//! * [`registry`] — the registry's `/k/*` and `/blobs/*` endpoints.
//! * [`compose`]  — what the compose file declares, and the `compose::*`
//!   calls an apply makes.
//! * [`origin`]   — who owns an `agents/<id>.md` (kit, worker, local …).
//! * [`merge`]    — three-way merge of locally edited kit files.
//! * [`plan`]     — install / update / removal plans (the contract with the
//!   UI and chat cards).
//! * [`store`]    — pending plans on disk.
//! * [`apply`]    — executing a reviewed plan.
//! * [`service`]  — the operations behind `directory::download-kit` and
//!   `directory::kits::*`.

pub mod apply;
pub mod compose;
pub mod kitref;
pub mod lock;
pub mod merge;
pub mod origin;
pub mod paths;
pub mod plan;
pub mod registry;
pub mod service;
pub mod store;

#[cfg(test)]
mod tests;
