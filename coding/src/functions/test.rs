use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;
use tokio::process::Command;

use crate::config::CodingConfig;
use crate::state;

pub fn build_handler(
    iii: Arc<III>,
    config: Arc<CodingConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let config = config.clone();
        Box::pin(async move { handle(&iii, &config, payload).await })
    }
}

pub async fn handle(
    iii: &III,
    config: &CodingConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    if let Some(worker_id) = payload.get("worker_id").and_then(|v| v.as_str()) {
        return test_worker(iii, config, worker_id).await;
    }

    let code = payload
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            IIIError::Handler(
                "provide either worker_id or code + language + test_code".to_string(),
            )
        })?;

    let language = payload
        .get("language")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: language".to_string()))?;

    let test_code = payload
        .get("test_code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: test_code".to_string()))?;

    test_inline(config, language, code, test_code).await
}

async fn test_worker(
    iii: &III,
    config: &CodingConfig,
    worker_id: &str,
) -> Result<Value, IIIError> {
    let worker_state = state::state_get(iii, "coding:workers", worker_id).await?;

    let language = worker_state
        .get("language")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("worker state missing language".to_string()))?;

    let workspace_path = worker_state
        .get("workspace_path")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| "");

    let effective_path = if workspace_path.is_empty() {
        format!("{}/{}", config.workspace_dir, worker_id)
    } else {
        workspace_path.to_string()
    };

    let timeout = std::time::Duration::from_millis(config.execute_timeout_ms);

    let (cmd_name, cmd_args) = match language {
        "rust" => ("cargo", vec!["test".to_string()]),
        "typescript" => ("npm", vec!["test".to_string()]),
        "python" => ("python3", vec!["-m".to_string(), "pytest".to_string()]),
        _ => {
            return Err(IIIError::Handler(format!(
                "unsupported language for testing: {}",
                language
            )));
        }
    };

    let run = tokio::time::timeout(
        timeout,
        Command::new(cmd_name)
            .args(&cmd_args)
            .current_dir(&effective_path)
            .output(),
    )
    .await
    .map_err(|_| IIIError::Handler("test execution timed out".to_string()))?
    .map_err(|e| IIIError::Handler(format!("failed to run test command: {}", e)))?;

    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run.stderr).to_string();
    let passed = run.status.success();
    let output = format!("{}\n{}", stdout, stderr);

    Ok(serde_json::json!({
        "passed": passed,
        "total": 0,
        "passed_count": 0,
        "failed_count": 0,
        "output": output,
    }))
}

async fn test_inline(
    config: &CodingConfig,
    language: &str,
    code: &str,
    test_code: &str,
) -> Result<Value, IIIError> {
    let exec_id = uuid::Uuid::new_v4().to_string();
    let work_dir = format!("{}/test_{}", config.workspace_dir, exec_id);

    std::fs::create_dir_all(&work_dir)
        .map_err(|e| IIIError::Handler(format!("failed to create work dir: {}", e)))?;

    let timeout = std::time::Duration::from_millis(config.execute_timeout_ms);

    let result = match language {
        "rust" => test_inline_rust(&work_dir, code, test_code, timeout).await,
        "typescript" => test_inline_typescript(&work_dir, code, test_code, timeout).await,
        "python" => test_inline_python(&work_dir, code, test_code, timeout).await,
        _ => Err(IIIError::Handler(format!(
            "unsupported language: {}",
            language
        ))),
    };

    let _ = std::fs::remove_dir_all(&work_dir);

    result
}

async fn test_inline_rust(
    work_dir: &str,
    code: &str,
    test_code: &str,
    timeout: std::time::Duration,
) -> Result<Value, IIIError> {
    let combined = format!("{}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n{}\n}}", code, test_code);
    let src_path = format!("{}/main.rs", work_dir);

    std::fs::write(&src_path, &combined)
        .map_err(|e| IIIError::Handler(format!("failed to write source: {}", e)))?;

    let run = tokio::time::timeout(
        timeout,
        Command::new("rustc")
            .arg("--test")
            .arg(&src_path)
            .arg("-o")
            .arg(format!("{}/test_bin", work_dir))
            .arg("--edition")
            .arg("2021")
            .output(),
    )
    .await
    .map_err(|_| IIIError::Handler("rust test compilation timed out".to_string()))?
    .map_err(|e| IIIError::Handler(format!("failed to compile tests: {}", e)))?;

    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr).to_string();
        return Ok(serde_json::json!({
            "passed": false,
            "total": 0,
            "passed_count": 0,
            "failed_count": 0,
            "output": stderr,
        }));
    }

    let test_run = tokio::time::timeout(
        timeout,
        Command::new(format!("{}/test_bin", work_dir)).output(),
    )
    .await
    .map_err(|_| IIIError::Handler("test execution timed out".to_string()))?
    .map_err(|e| IIIError::Handler(format!("failed to run test binary: {}", e)))?;

    let stdout = String::from_utf8_lossy(&test_run.stdout).to_string();
    let stderr = String::from_utf8_lossy(&test_run.stderr).to_string();
    let output = format!("{}\n{}", stdout, stderr);
    let passed = test_run.status.success();

    Ok(serde_json::json!({
        "passed": passed,
        "total": 0,
        "passed_count": 0,
        "failed_count": 0,
        "output": output,
    }))
}

async fn test_inline_typescript(
    work_dir: &str,
    code: &str,
    test_code: &str,
    timeout: std::time::Duration,
) -> Result<Value, IIIError> {
    let combined = format!("{}\n\n{}", code, test_code);
    let src_path = format!("{}/test.ts", work_dir);

    std::fs::write(&src_path, &combined)
        .map_err(|e| IIIError::Handler(format!("failed to write source: {}", e)))?;

    let runtime = if which_exists("bun") { "bun" } else { "node" };
    let args = if runtime == "bun" {
        vec!["run", &src_path]
    } else {
        vec!["--experimental-strip-types", &src_path]
    };

    let run = tokio::time::timeout(
        timeout,
        Command::new(runtime).args(&args).output(),
    )
    .await
    .map_err(|_| IIIError::Handler("test execution timed out".to_string()))?
    .map_err(|e| IIIError::Handler(format!("failed to run test: {}", e)))?;

    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run.stderr).to_string();
    let output = format!("{}\n{}", stdout, stderr);
    let passed = run.status.success();

    Ok(serde_json::json!({
        "passed": passed,
        "total": 0,
        "passed_count": 0,
        "failed_count": 0,
        "output": output,
    }))
}

async fn test_inline_python(
    work_dir: &str,
    code: &str,
    test_code: &str,
    timeout: std::time::Duration,
) -> Result<Value, IIIError> {
    let combined = format!("{}\n\n{}", code, test_code);
    let src_path = format!("{}/test_main.py", work_dir);

    std::fs::write(&src_path, &combined)
        .map_err(|e| IIIError::Handler(format!("failed to write source: {}", e)))?;

    let run = tokio::time::timeout(
        timeout,
        Command::new("python3")
            .arg("-m")
            .arg("pytest")
            .arg(&src_path)
            .arg("-v")
            .output(),
    )
    .await
    .map_err(|_| IIIError::Handler("test execution timed out".to_string()))?
    .map_err(|e| IIIError::Handler(format!("failed to run pytest: {}", e)))?;

    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run.stderr).to_string();
    let output = format!("{}\n{}", stdout, stderr);
    let passed = run.status.success();

    Ok(serde_json::json!({
        "passed": passed,
        "total": 0,
        "passed_count": 0,
        "failed_count": 0,
        "output": output,
    }))
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
