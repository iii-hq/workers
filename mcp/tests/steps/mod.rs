//! Step-definition modules. Each one corresponds to a `.feature` file
//! under `tests/features/`. The `#[given]` / `#[when]` / `#[then]`
//! macros register steps via `inventory` at compile time, so simply
//! `mod`-ing each file from here is enough to wire it up.
//!
//! Adding a feature: create `tests/features/<name>.feature`, add
//! `tests/steps/<name>.rs` with the step defs, and register it here.

pub mod errors;
pub mod initialize;
pub mod manifest;
pub mod notifications;
pub mod prompts;
pub mod resources;
pub mod tools;
