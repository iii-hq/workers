//! Live-engine guard for the @engine scenarios.
//!
//! `common::workers::register_all` registers a second, in-process
//! session-manager surface (the same `session::*` function ids) on the
//! engine it reaches. On an engine that already runs session-manager (a
//! developer's everyday engine on iii's default port) those ids are
//! taken: the scenarios' calls reach the real worker and write into its
//! real store, and a scenario that fails half-way leaves its sessions
//! there for good. So the scenarios only run on an engine that serves no
//! `session::*` function and has no worker named `session-manager`, and
//! only if the engine says so plainly: a listing that fails or comes back
//! in another shape skips them too (fail closed).
//!
//! The decision is taken twice: before the harness registers anything,
//! and again once it has, when every `session::*` function must be the
//! harness's own (a session-manager that reconnected in between, e.g.
//! across a compose restart, would own the ids instead).
//!
//! Kept pure (engine listings in, a verdict out) so it is unit-tested
//! without an engine, in `tests/engine_guard.rs`: the cucumber binary has
//! no test harness and never runs `#[test]`s.

use serde_json::Value;

/// The name session-manager connects with (`worker_metadata` in
/// `src/runtime.rs`; a compose container of that name gets it too).
const SESSION_MANAGER_WORKER: &str = "session-manager";

/// Every function the in-process surface registers (the handlers and the
/// `session::store::*` protocol) lives under this prefix.
const SESSION_FUNCTION_PREFIX: &str = "session::";

/// How to run the @engine scenarios when they skip.
const RUN_THEM: &str = "To run them, set III_ENGINE_WS_URL to a dedicated engine with no \
                        session-manager (e.g. a bare `iii --config <file>` on a free port).";

/// The worker name the harness connects with. Never `session-manager`,
/// and unique per process, so two runs on one engine do not trip the
/// engine's worker-name conflict (each sees the other's `session::*`
/// functions and skips instead).
pub fn harness_worker_name(pid: u32) -> String {
    format!("session-manager-bdd:{pid}")
}

/// Before the harness registers anything: why the @engine scenarios must
/// skip on the engine at `url`, or `None` to run them. `workers` and
/// `functions` are the outcomes of `engine::workers::list` and
/// `engine::functions::list` with `include_internal: true` (session-manager
/// registers its mutating functions as internal, and the engine hides
/// those otherwise).
pub fn skip_before_registering(
    url: &str,
    workers: &Result<Value, String>,
    functions: &Result<Value, String>,
) -> Option<String> {
    let workers = match listing(workers, "engine::workers::list", "workers") {
        Ok(list) => list,
        Err(why) => return Some(cannot_tell(url, &why)),
    };
    let functions = match listing(functions, "engine::functions::list", "functions") {
        Ok(list) => list,
        Err(why) => return Some(cannot_tell(url, &why)),
    };

    let mut evidence = Vec::new();
    if workers
        .iter()
        .any(|w| w["name"].as_str() == Some(SESSION_MANAGER_WORKER))
    {
        evidence.push(format!(
            "a worker named `{SESSION_MANAGER_WORKER}` is connected"
        ));
    }
    if let Some(served) = describe(&session_functions(functions, |_| true)) {
        evidence.push(served);
    }
    (!evidence.is_empty()).then(|| {
        format!(
            "iii engine at {url} already runs session-manager ({}). The engine-bound BDD \
             scenarios register their own session::* surface, so their calls would reach that \
             worker and write into its real store: every @engine scenario is skipped. {RUN_THEM}",
            evidence.join("; ")
        )
    })
}

/// After the harness registered its surface as worker `harness`: why the
/// @engine scenarios must skip, or `None` when every `session::*` function
/// on the engine is the harness's own.
pub fn skip_after_registering(
    url: &str,
    harness: &str,
    functions: &Result<Value, String>,
) -> Option<String> {
    let functions = match listing(functions, "engine::functions::list", "functions") {
        Ok(list) => list,
        Err(why) => return Some(cannot_tell(url, &why)),
    };
    let foreign = session_functions(functions, |worker| worker != Some(harness));
    describe(&foreign).map(|served| {
        format!(
            "a session-manager registered on the iii engine at {url} while the BDD harness was \
             starting ({served}), so the scenarios' calls would reach it and write into its real \
             store: every @engine scenario is skipped. {RUN_THEM}"
        )
    })
}

/// The array under `key`, or why the listing cannot be trusted.
fn listing<'a>(
    outcome: &'a Result<Value, String>,
    function: &str,
    key: &str,
) -> Result<&'a Vec<Value>, String> {
    match outcome {
        Err(e) => Err(format!("{function} failed ({e})")),
        Ok(value) => value[key]
            .as_array()
            .ok_or_else(|| format!("{function} answered without a `{key}` list")),
    }
}

fn cannot_tell(url: &str, why: &str) -> String {
    format!(
        "cannot tell whether the iii engine at {url} runs session-manager: {why}. Every @engine \
         scenario is skipped. {RUN_THEM}"
    )
}

/// `(function id, worker name)` of every `session::*` function whose
/// worker (`None` when the listing names none) matches `pick`.
fn session_functions(
    functions: &[Value],
    pick: impl Fn(Option<&str>) -> bool,
) -> Vec<(&str, Option<&str>)> {
    let mut served: Vec<_> = functions
        .iter()
        .filter_map(|f| {
            let id = f["function_id"].as_str()?;
            let worker = f["worker_name"].as_str();
            (id.starts_with(SESSION_FUNCTION_PREFIX) && pick(worker)).then_some((id, worker))
        })
        .collect();
    // Name `session::create` first when it is there: it is the call that
    // started every leaked session.
    served.sort_by_key(|(id, _)| (*id != "session::create", *id));
    served
}

fn describe(served: &[(&str, Option<&str>)]) -> Option<String> {
    let (id, worker) = served.first()?;
    let more = match served.len() - 1 {
        0 => String::new(),
        n => format!(" and {n} more `{SESSION_FUNCTION_PREFIX}*` function(s)"),
    };
    Some(format!(
        "worker `{}` already serves `{id}`{more}",
        worker.unwrap_or("<unnamed>")
    ))
}
