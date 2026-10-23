use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;

use crate::contract::{
    ConfigureRequestV1, CycleIdRequestV1, CycleListRequestV1, CycleStartRequestV1,
    EvaluationIdRequestV1, MonitorStateRequestV1, StepRequestV1, SweepEventV1,
};
use crate::runtime::Deps;

pub const CONFIGURE_ID: &str = "eval::configure";
pub const CONFIG_ID: &str = "eval::config";
pub const START_ID: &str = "eval::start";
pub const LIST_ID: &str = "eval::list";
pub const STATUS_ID: &str = "eval::status";
pub const RESULT_ID: &str = "eval::result";
pub const CANCEL_ID: &str = "eval::cancel";
pub const DELETE_ID: &str = "eval::delete";
pub const STEP_ID: &str = "eval::step";
pub const SWEEP_ID: &str = "eval::sweep";

pub fn register_all(iii: &Arc<IIIClient>, deps: &Deps) {
    let current = deps.clone();
    iii.register_function(
        CONFIGURE_ID,
        RegisterFunction::new_async(move |request: ConfigureRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::configure(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Choose the Harness model/provider the monitor uses for investigations. A new \
             selection is checked against router::models::list; credentials stay in the \
             providers and are never accepted here. An optional code_repository (an absolute \
             directory on this host, normally the iii workers repository) lets the \
             investigation read that code like a chat with the directory selected, with only \
             read-only functions allowed (coder::search, coder::tree, coder::read-file, \
             github::pr::list and engine::functions::info). An optional daily_cost_cap_usd \
             stops automatic analyses from starting an investigation for the rest of the UTC \
             day once the known investigation cost reaches it. Analyses already started keep the configuration they started with.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        CONFIG_ID,
        RegisterFunction::new_async(move |request: MonitorStateRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::monitor_state(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Read the monitor configuration (null until configured), the day's known \
             investigation cost against the cap with what one analysis has cost, and, with \
             check_providers, whether the Jev triage provider answers.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        START_ID,
        RegisterFunction::new_async(move |request: CycleStartRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_start(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Start one loop cycle from exactly one input: a free `objective`, or the \
             `execution_id` of an existing E2E execution. Both inputs create the same cycle \
             record in the loop's own history; follow it with eval::status and eval::result \
             and stop it with eval::cancel.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        LIST_ID,
        RegisterFunction::new_async(move |request: CycleListRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_list(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "List loop cycles, newest first, whichever input started them, each with its \
             history.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |request: CycleIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_status(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Read one loop cycle record (input, status, history) or null."),
    );

    let current = deps.clone();
    iii.register_function(
        RESULT_ID,
        RegisterFunction::new_async(move |request: CycleIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_result(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Read one loop cycle with its result (null until the cycle completes), or null.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        CANCEL_ID,
        RegisterFunction::new_async(move |request: CycleIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_cancel(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Cancel a loop cycle that is not finished; a finished cycle is left as is."),
    );

    let current = deps.clone();
    iii.register_function(
        DELETE_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::delete(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Delete a terminal analysis and its stored previews."),
    );

    let current = deps.clone();
    iii.register_function(
        STEP_ID,
        RegisterFunction::new_async(move |request: StepRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::step(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Internal durable analysis step.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        SWEEP_ID,
        RegisterFunction::new_async(move |_event: SweepEventV1| {
            let deps = current.clone();
            async move { crate::runtime::sweep(&deps).await.map_err(Error::from) }
        })
        .description("Internal recovery, deadline and retention sweep.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}
