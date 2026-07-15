//! memory-consolidate — scheduled hygiene sibling of the `memory` worker.
//!
//! Deterministic near-duplicate dedup over live memories, applied strictly
//! through the memory worker's public functions (`memory::supersede` +
//! `memory::save`): supersede-only, pinned untouchable, audit via memory's
//! own `memory::item-changed` events. Self-scheduled with catch-up-on-boot
//! semantics: the last completed pass is persisted in the state worker, so
//! a pass missed while the worker was down runs shortly after boot instead
//! of waiting a full interval.

pub mod config;
pub mod configuration;
pub mod consolidate;
pub mod functions;
pub mod manifest;
