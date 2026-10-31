//! Live-engine guard for the @engine scenarios.
//!
//! `common::workers::register_all` registers a second, in-process
//! session-manager surface (the same `session::*` function ids) on the
//! engine it reaches. On an engine that already runs session-manager (a
//! developer's everyday engine on iii's default port) those ids are
//! taken: the scenarios' calls reach the real worker and write into its
//! real store, and a scenario that fails half-way leaves its sessions
//! there for good. So the scenarios only run on an engine that serves no
//! `session::*` function and has no worker named `session-manager`.
//!
//! Kept pure (two engine listings in, a verdict out) so it is unit-tested
//! without an engine, in `tests/engine_guard.rs`: the cucumber binary has
//! no test harness and never runs `#[test]`s.

use serde_json::Value;

/// The name session-manager connects with (`worker_metadata` in
/// `src/runtime.rs`; a compose container of that name gets it too).
const SESSION_MANAGER_WORKER: &str = "session-manager";

/// Every function the in-process surface registers (the handlers and the
/// `session::store::*` protocol) lives under this prefix.
const SESSION_FUNCTION_PREFIX: &str = "session::";

/// What gives away a session-manager already on the engine, given its
/// `engine::workers::list` and `engine::functions::list` responses (the
/// latter with `include_internal: true`: session-manager registers its
/// mutating functions as internal, and the engine hides those otherwise).
/// `None` means the engine is free for the @engine scenarios.
pub fn session_manager_on_engine(workers: &Value, functions: &Value) -> Option<String> {
    let named = entries(workers, "workers")
        .filter(|w| w["name"].as_str() == Some(SESSION_MANAGER_WORKER))
        .count();
    let mut served: Vec<(&str, &str)> = entries(functions, "functions")
        .filter_map(|f| {
            let id = f["function_id"].as_str()?;
            id.starts_with(SESSION_FUNCTION_PREFIX)
                .then(|| (id, f["worker_name"].as_str().unwrap_or("?")))
        })
        .collect();
    // Name `session::create` first when it is there: it is the call that
    // started every leaked session.
    served.sort_by_key(|(id, _)| (*id != "session::create", *id));

    let mut evidence = Vec::new();
    if named > 0 {
        evidence.push(format!(
            "a worker named `{SESSION_MANAGER_WORKER}` is connected"
        ));
    }
    if let Some((id, worker)) = served.first() {
        let more = match served.len() - 1 {
            0 => String::new(),
            n => format!(" and {n} more `{SESSION_FUNCTION_PREFIX}*` function(s)"),
        };
        evidence.push(format!("worker `{worker}` already serves `{id}`{more}"));
    }
    (!evidence.is_empty()).then(|| evidence.join("; "))
}

fn entries<'a>(listing: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    listing[key].as_array().into_iter().flatten()
}
