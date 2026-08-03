//! Library surface for the `computer` worker: drive a full desktop on the iii
//! bus through a computer-use driver. The binary (`src/main.rs`) is a thin
//! boot sequence; everything testable lives here.

pub mod config;
pub mod configuration;
pub mod driver;
pub mod events;
pub mod functions;
pub mod manifest;
pub mod session;
pub mod ui;
