//! fp worker library surface: the ten pure transforms (`util`), the
//! trigger-binding guard (`condition`), the
//! worker-side pipeline (`pipe`), the system-prompt guidance hook
//! (`guidance`), and its `configuration`-worker knob (`config` /
//! `configuration`). The `fp` binary (src/main.rs) wires these onto the
//! bus; the lib target exists so `tests/` can exercise the public contract.

pub mod condition;
pub mod config;
pub mod configuration;
pub mod guidance;
pub mod pipe;
pub mod util;
