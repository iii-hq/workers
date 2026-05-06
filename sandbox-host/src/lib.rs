//! Pseudo-sandbox that maps `sandbox::*` calls to direct host operations.
//!
//! This worker exists so a solo-developer harness install can run the
//! `shell-filesystem` and `shell-bash` tools without standing up a real
//! sandbox stack. Every call hits the host filesystem / process table
//! with NO isolation, NO path allowlist, NO chroot.
//!
//! Multi-tenant or untrusted deployments must replace this with an
//! actual sandbox worker (containers, gVisor, firejail, etc.).

use std::path::Path;
use std::process::Stdio;

use iii_sdk::{
    ChannelReader, ChannelWriter, FunctionRef, IIIError, RegisterFunctionMessage,
    StreamChannelRef, III,
};
use serde_json::{json, Value};
use tokio::process::Command;

pub const HOST_SANDBOX_ID: &str = "host";

pub fn register(iii: &III) -> Vec<FunctionRef> {
    let mut refs = Vec::with_capacity(9);

    refs.push(register_create(iii));
    refs.push(register_list(iii));
    refs.push(register_stop(iii));
    refs.push(register_fs_ls(iii));
    refs.push(register_fs_read(iii));
    refs.push(register_fs_write(iii));
    refs.push(register_fs_mkdir(iii));
    refs.push(register_fs_stat(iii));
    refs.push(register_exec(iii));

    refs
}

fn require_str(payload: &Value, key: &str) -> Result<String, IIIError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required arg: {key}")))
}

fn register_create(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::create".into())
            .with_description("Host pseudo-sandbox: returns a fixed sandbox_id.".into()),
        |_payload: Value| async move {
            Ok::<_, IIIError>(json!({ "sandbox_id": HOST_SANDBOX_ID }))
        },
    ))
}

fn register_list(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::list".into())
            .with_description("List host sandboxes (always one).".into()),
        |_payload: Value| async move {
            Ok::<_, IIIError>(json!({
                "items": [{ "sandbox_id": HOST_SANDBOX_ID }]
            }))
        },
    ))
}

fn register_stop(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::stop".into())
            .with_description("No-op for host pseudo-sandbox.".into()),
        |_payload: Value| async move { Ok::<_, IIIError>(json!({ "ok": true })) },
    ))
}

fn register_fs_ls(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::fs::ls".into())
            .with_description("List directory entries on the host filesystem.".into()),
        |payload: Value| async move {
            let path = require_str(&payload, "path")?;
            let read = std::fs::read_dir(&path)
                .map_err(|e| IIIError::Handler(format!("read_dir {path}: {e}")))?;
            let mut entries: Vec<Value> = Vec::new();
            for r in read.flatten() {
                let name = r.file_name().to_string_lossy().into_owned();
                let is_dir = r.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let size = r.metadata().map(|m| m.len()).unwrap_or(0);
                entries.push(json!({ "name": name, "is_dir": is_dir, "size": size }));
            }
            Ok::<_, IIIError>(json!({ "entries": entries }))
        },
    ))
}

fn register_fs_read(iii: &III) -> FunctionRef {
    let iii_for_handler = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::fs::read".into())
            .with_description("Read a file from the host filesystem (streamed via channel).".into()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move {
                let path = require_str(&payload, "path")?;
                let bytes = std::fs::read(&path)
                    .map_err(|e| IIIError::Handler(format!("read {path}: {e}")))?;
                let channel = iii
                    .create_channel(None)
                    .await
                    .map_err(|e| IIIError::Handler(format!("create_channel: {e}")))?;
                let writer = ChannelWriter::new(iii.address(), &channel.writer_ref);
                writer
                    .write(&bytes)
                    .await
                    .map_err(|e| IIIError::Handler(format!("channel write: {e}")))?;
                writer
                    .close()
                    .await
                    .map_err(|e| IIIError::Handler(format!("channel close: {e}")))?;
                Ok::<_, IIIError>(json!({
                    "content": channel.reader_ref,
                    "size": bytes.len(),
                }))
            }
        },
    ))
}

fn register_fs_write(iii: &III) -> FunctionRef {
    let iii_for_handler = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::fs::write".into())
            .with_description("Write bytes to a file on the host filesystem.".into()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move {
                let path = require_str(&payload, "path")?;
                let parents = payload
                    .get("parents")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let channel_value = payload
                    .get("content")
                    .cloned()
                    .ok_or_else(|| IIIError::Handler("missing content channel".into()))?;
                let channel_ref: StreamChannelRef = serde_json::from_value(channel_value)
                    .map_err(|e| IIIError::Handler(format!("invalid content channel: {e}")))?;
                let reader = ChannelReader::new(iii.address(), &channel_ref);
                let bytes = reader
                    .read_all()
                    .await
                    .map_err(|e| IIIError::Handler(format!("channel drain: {e}")))?;
                if parents {
                    if let Some(parent) = Path::new(&path).parent() {
                        if !parent.as_os_str().is_empty() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| IIIError::Handler(format!("mkdir parents: {e}")))?;
                        }
                    }
                }
                std::fs::write(&path, &bytes)
                    .map_err(|e| IIIError::Handler(format!("write {path}: {e}")))?;
                Ok::<_, IIIError>(json!({ "bytes": bytes.len() }))
            }
        },
    ))
}

fn register_fs_mkdir(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::fs::mkdir".into())
            .with_description("Create a directory on the host filesystem.".into()),
        |payload: Value| async move {
            let path = require_str(&payload, "path")?;
            let parents = payload
                .get("parents")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let result = if parents {
                std::fs::create_dir_all(&path)
            } else {
                std::fs::create_dir(&path)
            };
            result.map_err(|e| IIIError::Handler(format!("mkdir {path}: {e}")))?;
            Ok::<_, IIIError>(json!({ "ok": true }))
        },
    ))
}

fn register_fs_stat(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::fs::stat".into())
            .with_description("Stat a file or directory on the host filesystem.".into()),
        |payload: Value| async move {
            let path = require_str(&payload, "path")?;
            let meta = std::fs::metadata(&path)
                .map_err(|e| IIIError::Handler(format!("stat {path}: {e}")))?;
            let name = Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            Ok::<_, IIIError>(json!({
                "name": name,
                "is_dir": meta.is_dir(),
                "size": meta.len(),
            }))
        },
    ))
}

fn register_exec(iii: &III) -> FunctionRef {
    iii.register_function((
        RegisterFunctionMessage::with_id("sandbox::exec".into())
            .with_description("Spawn a command directly on the host. NO isolation.".into()),
        |payload: Value| async move {
            let cmd = require_str(&payload, "cmd")?;
            let args: Vec<String> = payload
                .get("args")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let timeout_ms = payload
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(30_000);

            let child = Command::new(&cmd)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| IIIError::Handler(format!("spawn {cmd}: {e}")))?;

            let output = match tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                child.wait_with_output(),
            )
            .await
            {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(IIIError::Handler(format!("wait {cmd}: {e}"))),
                Err(_) => {
                    return Ok(json!({
                        "stdout": "",
                        "stderr": format!("timeout after {}ms", timeout_ms),
                        "exit_code": -1,
                        "timed_out": true,
                    }))
                }
            };

            Ok::<_, IIIError>(json!({
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "exit_code": output.status.code().unwrap_or(-1),
                "timed_out": false,
            }))
        },
    ))
}
