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

#[derive(Debug, Serialize, JsonSchema)]
pub struct PickOutput {
    pub ok: bool,
}
