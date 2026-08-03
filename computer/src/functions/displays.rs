//! `computer::displays` — list the local displays so a caller can pick which
//! one a native session drives.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::driver::DisplayInfo;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DisplaysInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DisplaysOutput {
    pub displays: Vec<DisplayInfo>,
}
