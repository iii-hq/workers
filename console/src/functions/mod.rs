//! Function registrations against the iii engine.
//!
//! `console` is mostly an HTTP server + WS proxy, so the SDK surface is
//! deliberately small: one health/identity function (`console::status`)
//! that returns the runtime knobs the worker booted with.

pub mod status;

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};

use crate::config::ConsoleConfig;
use status::{StatusInput, StatusOutput};

/// Register every `console::*` function. Called once from `main` after
/// `register_worker`. Each handler captures the runtime knobs without
/// re-parsing the YAML.
pub fn register_all(iii: &Arc<III>, config: &Arc<ConsoleConfig>, engine_url: &str) {
    register_status(iii, config, engine_url);
    tracing::info!("registered console::status");
}

fn register_status(iii: &Arc<III>, config: &Arc<ConsoleConfig>, engine_url: &str) {
    let cfg = config.clone();
    let engine_url = engine_url.to_string();
    iii.register_function(
        RegisterFunction::new_async("console::status", move |_: StatusInput| {
            let cfg = cfg.clone();
            let engine_url = engine_url.clone();
            async move {
                Ok::<_, IIIError>(StatusOutput {
                    http_port: cfg.http_port,
                    engine_url,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                })
            }
        })
        .description(
            "Return the console worker's runtime knobs: http_port, engine_url, and version.",
        ),
    );
}
