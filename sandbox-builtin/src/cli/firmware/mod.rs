//! Firmware and init binary management for iii-worker.
//!
//! The VMM (msb_krun) is compiled directly into the iii-worker binary.
//! libkrunfw (the guest kernel firmware) and iii-init (the guest init process)
//! are resolved via a two-stage chain, and fail closed (nothing is downloaded):
//!
//! 1. **Local resolution** -- env var, `~/.iii/lib/`, adjacent to binary, system paths
//! 2. **Embedded bytes** -- compile-time features (`embed-libkrunfw`, `embed-init`)
//!
//! Release builds embed both via `--features embed-init,embed-libkrunfw`
//! (build iii-init first with `scripts/build-init.sh`). A build without them
//! needs `III_LIBKRUNFW_PATH` / `III_INIT_PATH` or a local install.

pub mod constants;
pub(crate) mod download;
pub(crate) mod libkrunfw_bytes;
pub mod resolve;
pub(crate) mod symlinks;
