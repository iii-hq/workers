use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::assets::{javascript_response, static_asset};
use super::controller::{validate_stack_url, Controller};
use super::presenter::{
    execution_detail_value, load_execution_summaries, repository_url, validate_execution_id,
    MAX_EXECUTIONS,
};
use super::store::{read_metadata, read_report};
use super::{ApiError, DashboardArgs, RunRequest, RunSnapshot};
use crate::context::E2eContext;
use crate::scenarios::ScenarioId;

const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct AppState {
    controller: Arc<Controller>,
}

#[derive(Debug, Deserialize)]
struct CatalogQuery {
    url: Option<String>,
}

pub(super) async fn serve(args: DashboardArgs) -> Result<()> {
    if !args.listen.ip().is_loopback() {
        bail!("dashboard --listen must use a loopback address; use SSH port forwarding for remote access");
    }
    let listen = args.listen;
    let state = AppState {
        controller: Controller::new(args)?,
    };
    let app = Router::new()
        .route("/api/local/run", get(run_snapshot).post(start_run))
        .route("/api/local/run/cancel", axum::routing::post(cancel_run))
        .route("/api/local/catalog", get(catalog))
        .route("/data.js", get(benchmark_data))
        .route("/executions.js", get(execution_manifest))
        .route("/runs/:id", get(execution_detail))
        .fallback(get(static_asset))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("bind dashboard on {listen}"))?;
    println!("dashboard: http://{listen}/index.html");
    println!("press Ctrl+C to stop");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("serve local dashboard")?;
    Ok(())
}

async fn run_snapshot(State(state): State<AppState>) -> Result<Json<RunSnapshot>, ApiError> {
    state
        .controller
        .snapshot()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn start_run(
    State(state): State<AppState>,
    Json(request): Json<RunRequest>,
) -> Result<(StatusCode, Json<RunSnapshot>), ApiError> {
    state.controller.start(request).await?;
    let snapshot = state
        .controller
        .snapshot()
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

async fn cancel_run(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<RunSnapshot>), ApiError> {
    state.controller.cancel().await?;
    let snapshot = state
        .controller
        .snapshot()
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

async fn catalog(
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<Value>, ApiError> {
    let url = query
        .url
        .unwrap_or_else(|| state.controller.default_url().to_string());
    validate_stack_url(&url).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let context = E2eContext::connect(&url)
        .await
        .map_err(ApiError::internal)?;
    if !context
        .function_exists("harness::send")
        .await
        .map_err(ApiError::internal)?
    {
        context.shutdown().await;
        return Err(ApiError::bad_request(
            "connected iii stack does not expose harness::send",
        ));
    }
    if !context
        .function_exists("router::models::list")
        .await
        .map_err(ApiError::internal)?
    {
        context.shutdown().await;
        return Err(ApiError::bad_request(
            "connected Harness stack does not expose router::models::list; start its llm-router",
        ));
    }
    let models = crate::catalog::list(&context, None)
        .await
        .map_err(ApiError::internal);
    context.shutdown().await;
    let models = models?;
    if models.is_empty() {
        return Err(ApiError::bad_request(
            "the running Harness has no registered models",
        ));
    }
    let scenarios: Vec<_> = ScenarioId::ALL.iter().map(|value| value.as_str()).collect();
    Ok(Json(
        json!({ "url": url, "models": models, "scenarios": scenarios }),
    ))
}

async fn benchmark_data() -> Response {
    javascript_response("window.BENCHMARK_DATA = {};\n".into())
}

async fn execution_manifest(State(state): State<AppState>) -> Result<Response, ApiError> {
    let executions =
        load_execution_summaries(state.controller.runs_dir()).map_err(ApiError::internal)?;
    let last_update = executions
        .first()
        .and_then(|value| value.get("completed_at"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(javascript_response(format!(
        "window.HARNESS_EXECUTIONS = {};\n",
        json!({
            "schema_version": 4,
            "mode": "local",
            "last_update": last_update,
            "repo_url": repository_url(),
            "retention": { "summaries": MAX_EXECUTIONS, "details": MAX_EXECUTIONS },
            "executions": executions,
        })
    )))
}

async fn execution_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let id = id
        .strip_suffix(".json")
        .ok_or_else(|| ApiError::bad_request("execution detail must end in .json"))?
        .to_string();
    validate_execution_id(&id).map_err(ApiError::bad_request)?;
    let run_dir = state.controller.runs_dir().join(&id);
    let metadata = read_metadata(&run_dir)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "execution not found".into(),
        })?;
    let report = read_report(&run_dir)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "execution report not found".into(),
        })?;
    execution_detail_value(&metadata, &report)
        .map(Json)
        .map_err(ApiError::internal)
}
