pub mod admin;
pub mod files;
pub mod node;
pub mod peers;
pub mod share;

use std::process::Stdio;
use std::time::Duration;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use crate::config::{SharedConfig, WorkerConfig};

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

pub(crate) fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyInput {}

macro_rules! register_fn {
    ($iii:expr, $config:expr, $id:expr, $desc:expr, $input:ty, $handler:path) => {{
        let shared = $config.clone();
        $iii.register_function(
            $id,
            iii_sdk::RegisterFunction::new_async(move |input: $input| {
                let cfg = shared.load_full();
                async move {
                    $handler(&cfg, input)
                        .await
                        .map_err(iii_sdk::errors::Error::Handler)
                }
            })
            .description($desc),
        );
    }};
}
pub(crate) use register_fn;

pub fn catalog() -> Vec<FunctionSpec> {
    let mut specs = Vec::new();
    specs.extend(node::catalog());
    specs.extend(peers::catalog());
    specs.extend(share::catalog());
    specs.extend(files::catalog());
    specs.extend(admin::catalog());
    specs
}

pub fn register_all(iii: &IIIClient, config: SharedConfig) {
    node::register(iii, &config);
    peers::register(iii, &config);
    share::register(iii, &config);
    files::register(iii, &config);
    admin::register(iii, &config);
}

pub(crate) async fn run_output(config: &WorkerConfig, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(&config.tailscale_binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|error| format!("could not start Tailscale CLI: {error}"))?;
    let output = timeout(
        Duration::from_millis(config.command_timeout_ms),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        format!(
            "Tailscale CLI timed out after {} ms",
            config.command_timeout_ms
        )
    })?
    .map_err(|error| format!("Tailscale CLI failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Tailscale CLI exited with {}", output.status)
        });
    }
    Ok(output.stdout)
}

pub(crate) async fn run(config: &WorkerConfig, args: &[&str]) -> Result<(), String> {
    run_output(config, args).await.map(|_| ())
}

pub(crate) async fn run_text(config: &WorkerConfig, args: &[&str]) -> Result<String, String> {
    run_output(config, args)
        .await
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
}

pub(crate) async fn run_json(config: &WorkerConfig, args: &[&str]) -> Result<Value, String> {
    let output = run_output(config, args).await?;
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_slice(&output)
        .map_err(|error| format!("Tailscale returned invalid JSON: {error}"))
}

pub(crate) fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(crate) fn strings_at(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn bool_at(value: &Value, pointer: &str) -> bool {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn u64_at(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}
