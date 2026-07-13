//! `browser::pick::start` / `stop` — DevTools inspect mode for the human in
//! the console UI. Both are registered `internal`: picking is a human
//! gesture, not an agent function, so they stay out of agent tool lists.
//! The result arrives as a `browser::picked` trigger event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PickStartInput {
    pub session_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PickResolveInput {
    pub session_id: String,
    /// Viewport x of the click.
    pub x: f64,
    /// Viewport y of the click.
    pub y: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PickStopInput {
    /// Cancelling pick mode on an unknown session succeeds.
    pub session_id: String,
}

/// Neutral acknowledgement returned by the fire-and-forget internal
/// functions (pick start/stop/resolve, screencast start/stop): they emit
/// their result as a trigger event, so the direct return is just `{ ok }`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AckOutput {
    pub ok: bool,
}
