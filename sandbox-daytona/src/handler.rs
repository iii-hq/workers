//! Handler closures for every `sandbox::daytona::*` function. Each closure parses
//! its input as untyped JSON, does config-side validation (allowlist,
//! concurrency cap), then delegates to the narrow Daytona REST client. Failures
//! surface as `IIIError::Handler("[Sxxx] ...")` so callers can pattern-match
//! on the leading bracketed S-code.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::client::DaytonaClient;
use crate::config::Config;
use crate::{WorkerError, CAPABILITIES};

#[derive(Clone)]
pub struct HandlerCtx {
    pub config: Arc<Config>,
    pub client: Arc<DaytonaClient>,
    pub in_flight: Arc<AtomicUsize>,
}

impl HandlerCtx {
    pub fn new(config: Arc<Config>, client: Arc<DaytonaClient>) -> Self {
        Self {
            config,
            client,
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }
}

pub fn to_iii(err: WorkerError) -> IIIError {
    IIIError::Handler(err.to_string())
}

fn require_str(input: &Value, key: &str) -> Result<String, WorkerError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| WorkerError::BadInput(format!("missing string field `{key}`")))
}

pub async fn do_create(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let image = require_str(&input, "image")?;
    if !ctx.config.image_allowed(&image) {
        return Err(WorkerError::ImageNotAllowed(image));
    }
    let idle = input
        .get("idle_timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(ctx.config.default_idle_timeout_secs);

    // Reserve a concurrency slot before calling out. If we go over, give the
    // slot back and surface S400 immediately — don't queue.
    let prev = ctx.in_flight.fetch_add(1, Ordering::SeqCst);
    if prev >= ctx.config.max_concurrent_sandboxes {
        ctx.in_flight.fetch_sub(1, Ordering::SeqCst);
        return Err(WorkerError::ConcurrencyCapReached(prev));
    }

    match ctx.client.create(&image, idle).await {
        Ok(created) => Ok(json!({
            "sandbox_id": created.sandbox_id,
            "image": created.image,
            "started_at": created.started_at,
            "capabilities": CAPABILITIES,
        })),
        Err(e) => {
            ctx.in_flight.fetch_sub(1, Ordering::SeqCst);
            Err(e)
        }
    }
}

pub async fn do_exec(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let sandbox_id = require_str(&input, "sandbox_id")?;
    let cmd = require_str(&input, "cmd")?;
    let args: Vec<String> = input
        .get("args")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let timeout_ms = input.get("timeout_ms").and_then(Value::as_u64);
    let result = ctx
        .client
        .exec(&sandbox_id, &cmd, &args, timeout_ms)
        .await?;
    Ok(json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
        "timed_out": result.timed_out,
    }))
}

pub async fn do_stop(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let sandbox_id = require_str(&input, "sandbox_id")?;
    ctx.client.stop(&sandbox_id).await?;
    let _ = ctx
        .in_flight
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            Some(n.saturating_sub(1))
        });
    Ok(json!({}))
}

pub async fn do_list(ctx: &HandlerCtx, _input: Value) -> Result<Value, WorkerError> {
    // Best-effort upstream fetch. When the upstream answer is available we
    // also reconcile `in_flight` against it — providers like Daytona reap idle
    // sandboxes without notifying us, so the locally-incremented counter
    // would otherwise drift upward forever. Reconciling here makes `list`
    // both query and self-heal, at the cost of a tiny window where a
    // create racing this snapshot can briefly underreport.
    let (sandboxes, reconciled) = match ctx.client.list().await {
        Ok(items) => {
            let n = items.len();
            ctx.in_flight.store(n, Ordering::SeqCst);
            (items, true)
        }
        Err(_) => (Vec::new(), false),
    };
    let in_flight = ctx.in_flight.load(Ordering::SeqCst);
    let cap = ctx.config.max_concurrent_sandboxes;
    Ok(json!({
        "sandboxes": sandboxes,
        "in_flight": in_flight,
        "cap": cap,
        "remaining": cap.saturating_sub(in_flight),
        "reconciled": reconciled,
    }))
}

pub async fn do_snapshot(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let sandbox_id = require_str(&input, "sandbox_id")?;
    let snapshot_id = ctx.client.snapshot(&sandbox_id).await?;
    Ok(json!({ "snapshot_id": snapshot_id }))
}

pub async fn do_expose_port(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let sandbox_id = require_str(&input, "sandbox_id")?;
    let port = input
        .get("port")
        .and_then(Value::as_u64)
        .ok_or_else(|| WorkerError::BadInput("missing u16 port".into()))?;
    let port =
        u16::try_from(port).map_err(|_| WorkerError::BadInput("port out of range".into()))?;
    let url = ctx.client.expose_port(&sandbox_id, port).await?;
    Ok(json!({ "url": url }))
}

pub async fn do_fs_read(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let sandbox_id = require_str(&input, "sandbox_id")?;
    let path = require_str(&input, "path")?;
    let bytes = ctx.client.fs_read(&sandbox_id, &path).await?;
    Ok(json!({ "bytes_base64": B64.encode(bytes) }))
}

pub async fn do_fs_write(ctx: &HandlerCtx, input: Value) -> Result<Value, WorkerError> {
    let sandbox_id = require_str(&input, "sandbox_id")?;
    let path = require_str(&input, "path")?;
    let bytes_b64 = require_str(&input, "bytes_base64")?;
    let bytes = B64
        .decode(bytes_b64.as_bytes())
        .map_err(|e| WorkerError::BadInput(format!("invalid base64: {e}")))?;
    let mode = input
        .get("mode")
        .and_then(Value::as_u64)
        .map(|m| u32::try_from(m).unwrap_or(0o644));
    ctx.client
        .fs_write(&sandbox_id, &path, &bytes, mode)
        .await?;
    Ok(json!({}))
}
