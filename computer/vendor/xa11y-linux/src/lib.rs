//! Linux accessibility backend (vendored, patched xa11y-linux 0.15.2).
//!
//! AT-SPI2 introspection over D-Bus (`atspi`) with native EWMH/ICCCM window
//! management on X11 sessions — implements the core `Provider` trait. Pure
//! Rust: no system C library is linked. See `PATCHES.md` for the changes
//! relative to upstream.

#[cfg(target_os = "linux")]
mod atspi;

#[cfg(target_os = "linux")]
mod events;

#[cfg(target_os = "linux")]
mod scale;

#[cfg(target_os = "linux")]
mod session;

#[cfg(target_os = "linux")]
mod window;

#[cfg(target_os = "linux")]
pub use atspi::LinuxProvider;

#[cfg(not(target_os = "linux"))]
mod stub;

#[cfg(not(target_os = "linux"))]
pub use stub::LinuxProvider;
