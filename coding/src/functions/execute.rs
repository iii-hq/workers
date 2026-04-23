use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;
use tokio::process::Command;

use crate::config::CodingConfig;

pub fn build_handler(
    _iii: Arc<III>,
    config: Arc<CodingConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let config = config.clone();
        Box::pin(async move { handle(&config, payload).await })
    }
}

pub async fn handle(config: &CodingConfig, payload: Value) -> Result<Value, IIIError> {
    let code = payload
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: code".to_string()))?;

    let language = payload
        .get("language")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: language".to_string()))?;

    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.execute_timeout_ms);

    let input_json = payload
        .get("input")
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_default();

    let exec_id = uuid::Uuid::new_v4().to_string();
    let work_dir = format!("{}/exec_{}", config.workspace_dir, exec_id);

    std::fs::create_dir_all(&work_dir)
        .map_err(|e| IIIError::Handler(format!("failed to create work dir: {}", e)))?;

    let start = std::time::Instant::now();

    let result = match language {
        "rust" => execute_rust(&work_dir, code, &input_json, timeout_ms).await,
        "typescript" => execute_typescript(&work_dir, code, &input_json, timeout_ms).await,
        "python" => execute_python(&work_dir, code, &input_json, timeout_ms).await,
        _ => Err(IIIError::Handler(format!(
            "unsupported language: {}",
            language
        ))),
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    let _ = std::fs::remove_dir_all(&work_dir);

    match result {
        Ok((stdout, stderr, exit_code)) => Ok(serde_json::json!({
            "success": exit_code == 0,
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
            "duration_ms": duration_ms,
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "stdout": "",
            "stderr": e.to_string(),
            "exit_code": -1,
            "duration_ms": duration_ms,
        })),
    }
}

async fn execute_rust(
    work_dir: &str,
    code: &str,
    input_json: &str,
    timeout_ms: u64,
) -> Result<(String, String, i32), IIIError> {
    let src_path = format!("{}/main.rs", work_dir);
    let bin_path = format!("{}/main", work_dir);

    std::fs::write(&src_path, code)
        .map_err(|e| IIIError::Handler(format!("failed to write source: {}", e)))?;

    // kill_on_drop ensures the child rustc/binary is killed when the timeout
    // future is dropped. Without it, a timed-out compile leaves rustc running
    // in the background until it finishes on its own — exactly the kind of
    // zombie-accumulation that chews through a workspace over time.
    let mut compile_cmd = Command::new("rustc");
    compile_cmd
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path)
        .arg("--edition")
        .arg("2021")
        .kill_on_drop(true);

    let compile = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        compile_cmd.output(),
    )
    .await
    .map_err(|_| IIIError::Handler("compilation timed out".to_string()))?
    .map_err(|e| IIIError::Handler(format!("failed to run rustc: {}", e)))?;

    if !compile.status.success() {
        let stderr = String::from_utf8_lossy(&compile.stderr).to_string();
        return Ok(("".to_string(), stderr, compile.status.code().unwrap_or(1)));
    }

    let mut cmd = Command::new(&bin_path);
    cmd.kill_on_drop(true);
    if !input_json.is_empty() {
        cmd.env("III_INPUT", input_json);
    }

    let run = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), cmd.output())
        .await
        .map_err(|_| IIIError::Handler("execution timed out".to_string()))?
        .map_err(|e| IIIError::Handler(format!("failed to run binary: {}", e)))?;

    Ok((
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).to_string(),
        run.status.code().unwrap_or(1),
    ))
}

async fn execute_typescript(
    work_dir: &str,
    code: &str,
    input_json: &str,
    timeout_ms: u64,
) -> Result<(String, String, i32), IIIError> {
    let src_path = format!("{}/script.ts", work_dir);

    std::fs::write(&src_path, code)
        .map_err(|e| IIIError::Handler(format!("failed to write source: {}", e)))?;

    let runtime = if which_exists("bun") { "bun" } else { "node" };
    let args = if runtime == "bun" {
        vec!["run", &src_path]
    } else {
        vec!["--experimental-strip-types", &src_path]
    };

    let mut cmd = Command::new(runtime);
    cmd.args(&args).kill_on_drop(true);
    if !input_json.is_empty() {
        cmd.env("III_INPUT", input_json);
    }

    let run = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), cmd.output())
        .await
        .map_err(|_| IIIError::Handler("execution timed out".to_string()))?
        .map_err(|e| IIIError::Handler(format!("failed to run {}: {}", runtime, e)))?;

    Ok((
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).to_string(),
        run.status.code().unwrap_or(1),
    ))
}

async fn execute_python(
    work_dir: &str,
    code: &str,
    input_json: &str,
    timeout_ms: u64,
) -> Result<(String, String, i32), IIIError> {
    let src_path = format!("{}/script.py", work_dir);

    std::fs::write(&src_path, code)
        .map_err(|e| IIIError::Handler(format!("failed to write source: {}", e)))?;

    let mut cmd = Command::new("python3");
    cmd.arg(&src_path).kill_on_drop(true);
    if !input_json.is_empty() {
        cmd.env("III_INPUT", input_json);
    }

    let run = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), cmd.output())
        .await
        .map_err(|_| IIIError::Handler("execution timed out".to_string()))?
        .map_err(|e| IIIError::Handler(format!("failed to run python3: {}", e)))?;

    Ok((
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).to_string(),
        run.status.code().unwrap_or(1),
    ))
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
