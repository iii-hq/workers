//! Library surface for the `ios-simulator` worker: iOS Simulators on the iii
//! bus. The binary (`src/main.rs`) is a thin boot sequence; everything
//! testable lives here.

pub mod bridge;
pub mod config;
pub mod configuration;
pub mod events;
pub mod functions;
pub mod input;
pub mod manifest;
pub mod sim;
pub mod simctl;
pub mod tenant;
pub mod ui;
