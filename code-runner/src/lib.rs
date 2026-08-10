//! code-runner: run untrusted Node.js and Python in-process.
//!
//! One worker over two sandboxes — `iii-node-core` (V8 isolates) and, from
//! phase 5, `iii-python-core` (CPython on WebAssembly) — behind
//! sandbox-code-runner's wire contract, so a caller written against that
//! worker needs no changes. What differs is documented in the README: no
//! network, therefore no npm/pip, and no `/dev/kvm` requirement.

pub mod config;
pub mod error;
pub mod functions;
pub mod lang;
pub mod manager;
pub mod manifest;
pub mod node_bus;
pub mod python_bus;
pub mod translate;
pub mod truncate;
pub mod ui;
