//! Per-provider notifications pollers: each turns upstream messages into
//! calls of the object functions.
pub mod cf_queue;
pub mod pubsub;
pub mod sqs;
