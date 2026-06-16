//! Register the codex::* surface. Handlers parse at the unknown boundary and
//! delegate to the codex turn loop; schemas are published via request_format
//! so `iii trigger codex::run --help` prints the parameter table.

pub mod types;

use iii_sdk::{IIIError, RegisterFunction, III};
use serde_json::{json, Value};

use crate::codex;
use crate::configuration::ConfigCell;
use crate::state::{list_sessions, load_session, mark_error};
use types::{RunRequest, SessionIdRequest};

fn schema_value<T: schemars::JsonSchema>() -> Value {
    let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>();
    serde_json::to_value(root).expect("schema serializes")
}

pub fn register_all(iii: &III, cell: ConfigCell) {
    // codex::run — run a turn and wait for the result.
    {
        let iii_h = iii.clone();
        let cell_h = cell.clone();
        iii.register_function(
            "codex::run",
            RegisterFunction::new_async(move |req: RunRequest| {
                let iii_h = iii_h.clone();
                let cell_h = cell_h.clone();
                async move {
                    let cfg = { cell_h.read().await.clone() };
                    Ok::<Value, IIIError>(codex::run(iii_h, cfg, req).await)
                }
            })
            .request_format(schema_value::<RunRequest>())
            .description(
                "Run one Codex turn and wait for the result. Accepts `prompt` or a `messages` \
                 array plus a raw SDK `codex_config` pass-through; streams raw Codex events onto \
                 codex::events, AgentEvent frames onto agent::events, and returns \
                 {session_id, result, usage}.",
            ),
        );
    }

    // codex::start — fire-and-forget; progress on the streams.
    {
        let iii_h = iii.clone();
        let cell_h = cell.clone();
        iii.register_function(
            "codex::start",
            RegisterFunction::new_async(move |req: RunRequest| {
                let iii_h = iii_h.clone();
                let cell_h = cell_h.clone();
                async move {
                    let session_id = req
                        .session_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let mut started = req;
                    started.session_id = Some(session_id.clone());
                    let bg_iii = iii_h.clone();
                    let bg_id = session_id.clone();
                    let cfg = { cell_h.read().await.clone() };
                    // Supervise the run: run() persists terminal state itself,
                    // but if the task panics that never happens — the
                    // supervisor catches the JoinError and marks the session
                    // error so it can't stay stuck in `working`.
                    tokio::spawn(async move {
                        let run_iii = bg_iii.clone();
                        let inner = tokio::spawn(codex::run(run_iii, cfg, started));
                        match inner.await {
                            Ok(res) => {
                                if res.get("is_error").and_then(Value::as_bool) == Some(true) {
                                    mark_error(&bg_iii, &bg_id).await;
                                }
                            }
                            Err(e) => {
                                tracing::error!(session_id = %bg_id, error = %e, "codex::start task panicked");
                                mark_error(&bg_iii, &bg_id).await;
                            }
                        }
                    });
                    Ok::<Value, IIIError>(json!({ "session_id": session_id, "started": true }))
                }
            })
            .request_format(schema_value::<RunRequest>())
            .description(
                "Start a Codex turn and return immediately; watch codex::events / agent::events \
                 (group_id = session_id) for progress and turn_end.",
            ),
        );
    }

    // codex::stop — interrupt a live run.
    iii.register_function(
        "codex::stop",
        RegisterFunction::new_async(move |req: SessionIdRequest| async move {
            let stopped = codex::stop(&req.session_id).await;
            Ok::<Value, IIIError>(json!({
                "session_id": req.session_id,
                "stopped": stopped,
                "reason": if stopped { Value::Null } else { json!("no live run") },
            }))
        })
        .request_format(schema_value::<SessionIdRequest>())
        .description("Interrupt a live Codex run for a session."),
    );

    // codex::status — point-in-time session view.
    {
        let iii_h = iii.clone();
        iii.register_function(
            "codex::status",
            RegisterFunction::new_async(move |req: SessionIdRequest| {
                let iii_h = iii_h.clone();
                async move {
                    let record = load_session(&iii_h, &req.session_id).await.ok().flatten();
                    let live = codex::is_live(&req.session_id).await;
                    Ok::<Value, IIIError>(json!({
                        "session_id": req.session_id,
                        "live": live,
                        "record": record,
                    }))
                }
            })
            .request_format(schema_value::<SessionIdRequest>())
            .description("Point-in-time status of a Codex session."),
        );
    }

    // codex::sessions::list — every session this worker has run.
    {
        let iii_h = iii.clone();
        iii.register_function(
            "codex::sessions::list",
            RegisterFunction::new_async(move |_req: Value| {
                let iii_h = iii_h.clone();
                async move {
                    let sessions = list_sessions(&iii_h).await.unwrap_or_default();
                    Ok::<Value, IIIError>(json!({ "sessions": sessions }))
                }
            })
            .request_format(json!({ "type": "object", "properties": {} }))
            .description("List every Codex session this worker has run."),
        );
    }
}
