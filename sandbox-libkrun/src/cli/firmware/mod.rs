//! Firmware and init binary management for sandbox-libkrun.
//!
//! The VMM (msb_krun) is compiled directly into the sandbox-libkrun binary.
//! libkrunfw (the guest kernel firmware) and iii-init (the guest init process)
//! come from the compile-time embeds (`embed-libkrunfw`, `embed-init`) or a
//! local override, and fail closed (nothing is downloaded):
//!
//! - libkrunfw: `III_LIBKRUNFW_PATH`, `~/.iii/lib/` (where the embedded copy is
//!   extracted), system paths, then the embedded bytes.
//! - iii-init: the embedded bytes, then `III_INIT_PATH`.
//!
//! Release builds embed both via `--features embed-init,embed-libkrunfw`
//! (build iii-init first with `scripts/build-init.sh`). A build without them
//! needs `III_INIT_PATH`, and `III_LIBKRUNFW_PATH` or a system libkrunfw.

pub mod constants;
pub(crate) mod libkrunfw_bytes;
pub(crate) mod provision;
pub mod resolve;
pub(crate) mod symlinks;
