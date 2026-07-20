//! fp worker library surface: the ten pure transforms (`util`), the
//! worker-side pipeline (`pipe`), and the system-prompt guidance hook
//! (`guidance`). The `fp` binary (src/main.rs) wires these onto the bus;
//! the lib target exists so `tests/` can exercise the public contract.

pub mod guidance;
pub mod pipe;
pub mod util;
