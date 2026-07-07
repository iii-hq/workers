//! Transport adapters implementing [`crate::adapter::QueueAdapter`].

pub mod builtin;
#[cfg(feature = "test-adapters")]
pub mod memory;
#[cfg(feature = "rabbitmq")]
pub mod rabbitmq;
pub mod redis;
