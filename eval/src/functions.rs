use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;

use crate::contract::{
    AnalyzeSessionRequestV1, AttachValidationRequestV1, ConfigureRequestV1, EvalListRequestV1,
    EvaluationIdRequestV1, MonitorStateRequestV1, ProposeValidationRequestV1, StepRequestV1,
    SweepEventV1, WakeEventV1,
};
use crate::runtime::Deps;

pub const CONFIGURE_ID: &str = "eval::configure";
pub const CONFIG_ID: &str = "eval::config";
pub const ANALYZE_SESSION_ID: &str = "eval::analyze-session";
pub const LIST_ID: &str = "eval::list";
pub const STATUS_ID: &str = "eval::status";
pub const RESULT_ID: &str = "eval::result";
pub const CANCEL_ID: &str = "eval::cancel";
pub const DELETE_ID: &str = "eval::delete";
pub const ATTACH_VALIDATION_ID: &str = "eval::attach-validation";
pub const PROPOSE_VALIDATION_ID: &str = "eval::propose-validation";
pub const STEP_ID: &str = "eval::step";
pub const WAKE_ID: &str = "eval::on-turn-completed";
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
            "Choose the Harness model/provider the session monitor uses for investigations and \
             enable or pause automatic observation. A new selection is checked against \
             router::models::list; credentials stay in the providers and are never accepted \
             here. An optional code_repository (an absolute directory on this host, normally \
             the iii workers repository) lets the investigation read that code like a chat \
             with the directory selected, and for now with every function allowed. Analyses \
             already admitted keep the configuration they started with.",
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
            "Read the monitor configuration (null until configured), whether the \
             harness::turn-completed observation trigger was requested, the latest capacity \
             rejection and, with check_providers, whether the Jev triage provider answers.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        ANALYZE_SESSION_ID,
        RegisterFunction::new_async(move |request: AnalyzeSessionRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::analyze_session(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Analyze the latest definitive turn of a finished root Harness session. Returns the \
             existing analysis of that turn unless reanalyze is true; reanalysis needs the \
             previous analysis to be terminal. Works while automatic observation is paused.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        LIST_ID,
        RegisterFunction::new_async(move |request: EvalListRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::list(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "List recent monitor analyses as compact records, newest first, without evidence.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::status(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Read one analysis record (status, counters, failure stage) or null."),
    );

    let current = deps.clone();
    iii.register_function(
        RESULT_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::result(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Read an analysis with its captured evidence, deterministic diagnostics, Jev triage, \
             suggestions, the monitor's own consumption and linked E2E executions, or null. A \
             completed analysis is not a validated improvement.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        CANCEL_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cancel(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Cancel an analysis: signals its Jev call and stops its investigation session. Never \
             touches the observed session or the captured evidence.",
        ),
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
        .description(
            "Delete a terminal analysis and its stored previews. The turn stays marked as \
             analyzed until retention, so it is not re-admitted automatically.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        ATTACH_VALIDATION_ID,
        RegisterFunction::new_async(move |request: AttachValidationRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::attach_validation(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Link a baseline and a candidate E2E execution (read through \
             e2e::dashboard::execution-get) to one suggestion of a terminal analysis. The link \
             records report availability; it is not a verdict and starts no campaign.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        PROPOSE_VALIDATION_ID,
        RegisterFunction::new_async(move |request: ProposeValidationRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::propose_validation(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Ask Jev to pick the baseline and candidate E2E executions (listed through \
             e2e::dashboard::executions-list) that fit one suggestion's validation plan. Code \
             pre-filters the comparable pairs and counts the runs it leaves out; Jev only \
             chooses among them. Nothing is attached and no campaign starts; the call's Jev \
             usage is added to the analysis. Its confidence is not proof: check the runs.",
        ),
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
        WAKE_ID,
        RegisterFunction::new_async(move |event: WakeEventV1| {
            let deps = current.clone();
            async move {
                crate::runtime::wake(&deps, event)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Internal harness turn-completed admission and wake-up.")
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
