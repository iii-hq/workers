//! `sandbox` worker — caller-facing `sandbox::*` ids.
//!
//! Mirrors the `provider-router` shape: the router owns the bare namespace
//! and dispatches by the `provider` field to `sandbox::provider::<name>::*`.
//! When `provider` is absent, `DEFAULT_PROVIDER = "local"` is used. The
//! `local` provider is reserved for the engine-shipped `iii-sandbox`
//! (libkrun microVM); when no adapter is registered for it, calls return
//! the engine's standard "function not found" error.

pub mod config;
pub mod register;
