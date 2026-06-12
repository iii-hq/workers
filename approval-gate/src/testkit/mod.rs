//! Engine-free test doubles. `#[cfg(test)]`-free on purpose: the
//! integration suite (tests/integration.rs) uses these too.

pub mod fake_bus;

pub use fake_bus::{FakeBus, MemoryState, RecordedCall};
