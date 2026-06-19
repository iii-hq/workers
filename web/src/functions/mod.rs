//! Function registration for the web worker.

use std::sync::Arc;

use iii_sdk::III;

use crate::config::SharedConfig;

pub mod fetch;

pub fn register_all(iii: &Arc<III>, shared: &SharedConfig) {
    fetch::register(iii, shared);
}
