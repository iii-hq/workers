//! Test support. Compiled unconditionally (tiny) so `tests/` and downstream
//! worker crates can drive the router without the engine.
pub mod fake_bus;
pub mod fake_channels;
pub mod scripted_provider;
