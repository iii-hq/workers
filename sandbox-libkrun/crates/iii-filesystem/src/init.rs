//! Embedded init binary bytes.
//!
//! With `--features embed-init`, `INIT_BYTES` holds the guest init that
//! `scripts/build-init.sh` built, and the build fails when that init is
//! missing or older than its sources. Otherwise it is empty, and VMs
//! fall back to the default libkrun init.

/// The embedded init binary. Empty when the `embed-init` feature is off.
#[cfg(has_init_binary)]
pub const INIT_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/iii-init"));

#[cfg(not(has_init_binary))]
pub const INIT_BYTES: &[u8] = &[];

/// Returns `true` if the init binary is embedded (non-empty `INIT_BYTES`).
///
/// Since `INIT_BYTES` is a compile-time constant, this is resolved at compile time
/// and the compiler optimizes away all dead branches guarded by this function.
pub const fn has_init() -> bool {
    !INIT_BYTES.is_empty()
}
