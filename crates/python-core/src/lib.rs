//! iii-python-core: run untrusted one-shot Python in CPython-on-WebAssembly.
//!
//! Everything here is bus-free. `Manager` and `Runner` take a `RunSpec` and
//! return a `RunOutcome`; nothing in this tree names an `iii-sdk` type, so
//! nothing in it pins an SDK version. The wire types, the function
//! registration and the error conversion live with the hosting worker.

pub mod artifact;
pub mod config;
pub mod error;
pub mod manager;
pub mod runner;
