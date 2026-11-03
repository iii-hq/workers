/// The service function this worker registers. The builtin registers the BARE
/// id `publish` (its `#[service(name = "pubsub")]` prefix is discarded by the
/// macro — engine/function-macros/src/lib.rs:243); the stream bridge calls
/// `publish` (engine/src/workers/stream/adapters/bridge.rs:102). Never rename.
///
/// Subscribers observe it with a filter on `input.topic`.
pub const PUBLISH_FUNCTION_ID: &str = "publish";

pub mod adapters;
pub mod boot;
pub mod config;
pub mod configuration;
pub mod deprecation;
pub mod hub;
pub mod manifest;
