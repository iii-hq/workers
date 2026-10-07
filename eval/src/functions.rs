use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;

use crate::contract::{
    AnalyzeSessionRequestV1, AttachValidationRequestV1, ConfigureRequestV1, EvalListRequestV1,
    EvaluationIdRequestV1, MonitorStateRequestV1, ProposeValidationRequestV1, RecurrenceRequestV1,
    ReproduceRequestV1, ReviewRequestV1, ReviewsRequestV1, StartValidationRequestV1, StepRequestV1,
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
pub const START_VALIDATION_ID: &str = "eval::start-validation";
pub const REVIEW_ID: &str = "eval::review";
pub const REVIEWS_ID: &str = "eval::reviews";
pub const RECURRENCE_ID: &str = "eval::recurrence";
pub const REPRODUCE_ID: &str = "eval::reproduce";
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
             with the directory selected, and for now with every function allowed. An optional \
             daily_cost_cap_usd pauses automatic observation for the rest of the UTC day once the \
             known investigation cost reaches it (manual analyses are never refused). Analyses \
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
             harness::turn-completed observation trigger was requested, the latest rejection \
             (capacity or daily cost cap), the day's known investigation cost against the cap with \
             what one analysis has cost, and, with check_providers, whether the Jev triage \
             provider answers.",
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
            "List recent monitor analyses as compact records, newest first, without evidence; \
             observation_key keeps only the analyses of one observed turn.",
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
             records report availability and, when the suggestion's scenario is known, the \
             evidence computed from the runs; it is not a verdict and starts no campaign.",
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
        START_VALIDATION_ID,
        RegisterFunction::new_async(move |request: StartValidationRequestV1| {
            let deps = current.clone();
            async move {
                crate::validation::start_validation(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Explicitly start a baseline and a candidate E2E execution for one suggestion. Both \
             commits (candidate_ref, and baseline_ref or the merge base with origin/main) are \
             resolved with git in the configured code_repository and must be on a remote branch; \
             each stack is the E2E's `harness` template with only the Harness pinned to its commit, \
             run in Docker (runs 1-20, default 5). The criterion is registered before anything \
             starts; the sweep attaches the pair when both finish and computes the evidence. \
             Spends model tokens, and never runs unless called. With dry_run: true it only \
             resolves and checks the refs, scenario and runs and answers with the resolved \
             commits (`baseline`, `candidate`, `warnings`); it registers nothing and calls no \
             E2E function.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        REVIEW_ID,
        RegisterFunction::new_async(move |request: ReviewRequestV1| {
            let deps = current.clone();
            async move {
                crate::review::review(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Record what a person decided about a suggestion of a terminal analysis: move its \
             lifecycle (new, accepted, in_progress, shipped, rejected, duplicate), register the \
             criterion a validation is judged by, or record a verdict. A verdict of \
             validated_improvement is refused unless the criterion was registered before the \
             first attached pair or run and both sides have min_runs completed runs. The author \
             is `by`, else the eval host's user, else the caller's identity.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        REVIEWS_ID,
        RegisterFunction::new_async(move |request: ReviewsRequestV1| {
            let deps = current.clone();
            async move {
                crate::review::reviews(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "List the stored suggestion reviews (lifecycle, criterion, run, evidence, verdict) \
             and, per analysis with suggestions, how many sit at each lifecycle status.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        RECURRENCE_ID,
        RegisterFunction::new_async(move |request: RecurrenceRequestV1| {
            let deps = current.clone();
            async move {
                crate::review::recurrence(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "For a suggestion marked shipped with its version, how often its patterns were found \
             per analysis on Harness versions before it against from it on. Analyses without a \
             semantic Harness version are left out and counted.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        REPRODUCE_ID,
        RegisterFunction::new_async(move |request: ReproduceRequestV1| {
            let deps = current.clone();
            async move {
                crate::reproduce::reproduce(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Replay the decision point of a suggestion: rebuild the request the model received at \
             that step of the observed turn (optionally with edits that stand for the proposed \
             change), sample the next reply N times (default 20) without running any function, \
             and read the suggestion's signal in each reply. Spends model tokens; with dry_run it \
             only rebuilds the request, checks its fidelity against the recorded usage and \
             reports the original reply. extend adds replies to an earlier reproduction, or with \
             samples 0 finishes one that failed. Results land in eval::result reviews.",
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
