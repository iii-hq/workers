//! Function registrations against the iii engine.
//!
//! `console` is mostly an HTTP server + WS proxy, so the SDK surface is
//! deliberately small: one health/identity function (`console::status`)
//! that returns the runtime knobs the worker booted with, plus the
//! injectable-UI debug surface (`console::ui-manifest`).

pub mod status;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::ConsoleConfig;
use crate::ui_assets::{ManifestAsset, UiRegistry};
use status::{StatusInput, StatusOutput};

/// Register every `console::*` function. Called once from `main` after
/// `register_worker` (and after `ui_assets::start`, which owns the
/// trigger-type registrations — types register before functions). Each
/// handler captures the runtime knobs without re-parsing the YAML.
pub fn register_all(
    iii: &Arc<IIIClient>,
    config: &Arc<ConsoleConfig>,
    engine_url: &str,
    ui: Option<Arc<UiRegistry>>,
) {
    register_status(iii, config, engine_url);
    register_ui_manifest(iii, ui);
    tracing::info!("registered console::status, console::ui-manifest");
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UiManifestInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UiManifestOutput {
    /// `true` when the `injectable_ui` kill switch is off — the trigger
    /// types are unregistered and `/ui` + `/vendor` are not served.
    pub disabled: bool,
    pub assets: Vec<ManifestAsset>,
}

/// `console::ui-manifest` — the authoritative *loadable* asset set (only
/// the console knows fetch results and hashes). Debug surface; the SPA
/// loader itself is fed by `console:assets` pushes, never this.
fn register_ui_manifest(iii: &Arc<IIIClient>, ui: Option<Arc<UiRegistry>>) {
    iii.register_function(
        "console::ui-manifest",
        RegisterFunction::new_async(move |_: UiManifestInput| {
            let ui = ui.clone();
            async move {
                Ok::<_, Error>(match ui {
                    Some(registry) => UiManifestOutput {
                        disabled: false,
                        assets: registry.manifest(),
                    },
                    None => UiManifestOutput {
                        disabled: true,
                        assets: Vec::new(),
                    },
                })
            }
        })
        .description(
            "List the injected console UI assets currently loadable: path, kind \
             (script/style), content hash, and style-lint warnings.",
        )
        // console-only plumbing, like console::status.
        .metadata(serde_json::json!({ "internal": true })),
    );
}

fn register_status(iii: &Arc<IIIClient>, config: &Arc<ConsoleConfig>, engine_url: &str) {
    let cfg = config.clone();
    let engine_url = engine_url.to_string();
    iii.register_function(
        "console::status",
        RegisterFunction::new_async(move |_: StatusInput| {
            let cfg = cfg.clone();
            let engine_url = engine_url.clone();
            async move {
                Ok::<_, Error>(StatusOutput {
                    http_port: cfg.http_port,
                    engine_url,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                })
            }
        })
        .description(
            "Return the console worker's runtime knobs: http_port, engine_url, and version.",
        )
        // console-only plumbing; no other worker (e.g. harness) needs to
        // discover or call it.
        .metadata(serde_json::json!({ "internal": true })),
    );
}
