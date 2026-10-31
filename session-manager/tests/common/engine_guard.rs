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
//! The decision is taken before the harness registers anything, again
//! once it has, and before every @engine scenario: each time after the
//! first, every `session::*` function must be the harness's own (a
//! session-manager that (re)connected in between, e.g. across an engine
//! restart that makes the harness replay its registrations, would own the
//! ids instead).
//!
//! Kept pure (engine listings in, a verdict out) so it is unit-tested
//! without an engine, in `tests/engine_guard.rs`: the cucumber binary has
//! no test harness and never runs `#[test]`s.

use iii_sdk::runtime::IIIConnectionState;
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

/// After the reachability probe (`engine::workers::list`, retried while
/// the client connects): why to skip now, or `None` to go on. A client
/// that is connected, or that the engine refused (`Failed`), reached an
/// engine it cannot inspect; any other state means no engine answered.
pub fn skip_after_probe(
    url: &str,
    state: IIIConnectionState,
    workers: &Result<Value, String>,
) -> Option<String> {
    let Err(e) = workers else {
        return None;
    };
    Some(match state {
        IIIConnectionState::Connected => {
            cannot_tell(url, &format!("engine::workers::list failed ({e})"))
        }
        IIIConnectionState::Failed => cannot_tell(
            url,
            &format!("the engine refused the harness's connection ({e})"),
        ),
        _ => format!(
            "iii engine not reachable at {url}; \
             set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
        ),
    })
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
    let workers = match listing(workers, "engine::workers::list", "workers", "name") {
        Ok(list) => list,
        Err(why) => return Some(cannot_tell(url, &why)),
    };
    let functions = match listing(
        functions,
        "engine::functions::list",
        "functions",
        "function_id",
    ) {
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

/// Once the harness registered its surface as worker `harness` (and again
/// before every @engine scenario): why the @engine scenarios must skip
/// from here on, or `None` when every `session::*` function on the engine
/// is the harness's own.
pub fn skip_after_registering(
    url: &str,
    harness: &str,
    functions: &Result<Value, String>,
) -> Option<String> {
    let functions = match listing(
        functions,
        "engine::functions::list",
        "functions",
        "function_id",
    ) {
        Ok(list) => list,
        Err(why) => return Some(cannot_tell(url, &why)),
    };
    let foreign = session_functions(functions, |worker| worker != Some(harness));
    describe(&foreign).map(|served| {
        format!(
            "another worker serves session::* functions on the iii engine at {url} next to the \
             BDD harness ({served}): a session-manager (re)connected after the harness's first \
             check, so the scenarios' calls could reach it and write into its real store. Every \
             @engine scenario from here on is skipped. {RUN_THEM}"
        )
    })
}

/// The array under `key`, every row of it carrying a string `field`, or
/// why the listing cannot be trusted.
fn listing<'a>(
    outcome: &'a Result<Value, String>,
    function: &str,
    key: &str,
    field: &str,
) -> Result<&'a Vec<Value>, String> {
    let value = outcome
        .as_ref()
        .map_err(|e| format!("{function} failed ({e})"))?;
    let rows = value[key]
        .as_array()
        .ok_or_else(|| format!("{function} answered without a `{key}` list"))?;
    match rows.iter().find(|row| !row[field].is_string()) {
        Some(row) => Err(format!(
            "{function} listed an entry without a string `{field}`: {row}"
        )),
        None => Ok(rows),
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
