//! Function registration for the web worker.

use std::sync::Arc;

use iii_sdk::IIIClient;

use crate::config::SharedConfig;

pub mod fetch;

pub fn register_all(iii: &Arc<IIIClient>, shared: &SharedConfig) {
    fetch::register(iii, shared);
}
