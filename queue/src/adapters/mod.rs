//! Transport adapters implementing [`crate::adapter::QueueAdapter`].

pub mod builtin;
#[cfg(feature = "rabbitmq")]
pub mod rabbitmq;
pub mod redis;
