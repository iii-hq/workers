use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerCreateParams {
    pub language: String,
    pub code: String,
    pub function_name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerStopParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpawnedWorker {
    pub id: String,
    pub language: String,
    pub function_name: String,
    pub temp_dir: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerCreateResult {
    pub id: String,
    pub function_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerStopResult {
    pub id: String,
    pub message: String,
}

fn js_string_literal(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

pub struct WorkerManager {
    engine_url: String,
    workers: Arc<Mutex<HashMap<String, (SpawnedWorker, Child)>>>,
}

impl WorkerManager {
    pub fn new(engine_url: String) -> Self {
        Self {
            engine_url,
            workers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_worker(
        &self,
        params: WorkerCreateParams,
    ) -> Result<WorkerCreateResult, String> {
        // Full UUID (not truncated) — truncating to 8 hex chars gave us 32
        // bits of entropy, and a collision in the HashMap::insert path
        // below caused the replaced entry's Child to drop, which
        // kill_on_drop(true) turns into SIGKILL on an unrelated worker.
        let worker_id = format!("worker-{}", Uuid::new_v4());

        // Validate language BEFORE creating the temp dir so unsupported
        // values don't leave iii-* directories behind in the system temp.
        let (file_name, code) = match params.language.as_str() {
            "node" | "javascript" | "js" => {
                let code = self.generate_node_worker(&params);
                ("index.mjs", code)
            }
            "python" | "py" => {
                let code = self.generate_python_worker(&params);
                ("main.py", code)
            }
            _ => return Err(format!("Unsupported language: {}", params.language)),
        };

        let temp_dir = std::env::temp_dir().join(format!("iii-{}", &worker_id));
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let file_path = temp_dir.join(file_name);
        if let Err(e) = tokio::fs::write(&file_path, &code).await {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!("Failed to write worker file: {}", e));
        }

        let mut child = match self
            .spawn_worker(&params.language, &temp_dir, file_name)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(e);
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(format!("Worker exited immediately with status: {}", status));
            }
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(format!("Failed to check worker status: {}", e));
            }
            Ok(None) => {}
        }

        let pid = child.id().unwrap_or(0);

        // Drain stderr into the worker-manager log. Without this the child
        // blocks once the stderr pipe buffer fills (~64 KiB on Linux).
        if let Some(stderr) = child.stderr.take() {
            let wid = worker_id.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(worker_id = %wid, stderr = %line, "worker stderr");
                }
            });
        }

        let spawned = SpawnedWorker {
            id: worker_id.clone(),
            language: params.language.clone(),
            function_name: params.function_name.clone(),
            temp_dir: temp_dir.to_string_lossy().to_string(),
            pid,
        };

        self.workers
            .lock()
            .await
            .insert(worker_id.clone(), (spawned, child));

        tracing::info!(
            worker_id = %worker_id,
            function_name = %params.function_name,
            pid = %pid,
            "Spawned worker"
        );

        Ok(WorkerCreateResult {
            id: worker_id,
            function_name: params.function_name,
            message: "Worker created and connecting to iii-engine".to_string(),
        })
    }

    pub async fn stop_worker(&self, params: WorkerStopParams) -> Result<WorkerStopResult, String> {
        // Drop the map mutex before awaiting kill/rmdir — holding it would
        // block every concurrent create_worker/stop_worker for the
        // duration of I/O.
        let entry = {
            let mut workers = self.workers.lock().await;
            workers.remove(&params.id)
        };

        let Some((info, mut child)) = entry else {
            return Err(format!("Worker not found: {}", params.id));
        };

        if let Err(e) = child.kill().await {
            tracing::warn!(worker_id = %params.id, error = %e, "Failed to kill worker process");
        }

        if let Err(e) = tokio::fs::remove_dir_all(&info.temp_dir).await {
            tracing::warn!(worker_id = %params.id, error = %e, "Failed to remove temp dir");
        }

        tracing::info!(worker_id = %params.id, "Stopped worker");

        Ok(WorkerStopResult {
            id: params.id,
            message: "Worker stopped and cleaned up".to_string(),
        })
    }

    fn generate_node_worker(&self, params: &WorkerCreateParams) -> String {
        let engine_url = js_string_literal(&self.engine_url);
        let function_name = js_string_literal(&params.function_name);
        let description = js_string_literal(
            params
                .description
                .as_deref()
                .unwrap_or("Auto-generated function"),
        );

        format!(
            r#"import {{ registerWorker, Logger }} from 'iii-sdk'

const iii = registerWorker({engine_url})
const logger = new Logger()

const handler = {code}

iii.registerFunction({{ id: {function_name}, description: {description} }}, handler)

logger.info('Function registered: ' + {function_name})

process.on('SIGTERM', () => {{
  logger.info('Worker shutting down')
  process.exit(0)
}})
process.on('SIGINT', () => {{
  logger.info('Worker interrupted')
  process.exit(0)
}})
"#,
            engine_url = engine_url,
            code = params.code,
            function_name = function_name,
            description = description,
        )
    }

    fn generate_python_worker(&self, params: &WorkerCreateParams) -> String {
        let engine_url = js_string_literal(&self.engine_url);
        let function_name = js_string_literal(&params.function_name);
        let description = js_string_literal(
            params
                .description
                .as_deref()
                .unwrap_or("Auto-generated function"),
        );

        format!(
            r#"import asyncio
import signal
from iii_sdk import register_worker, Logger

iii = register_worker({engine_url})
logger = Logger()

{code}

iii.register_function({function_name}, handler, description={description})

def shutdown(sig, frame):
    logger.info('Worker shutting down')
    exit(0)

signal.signal(signal.SIGTERM, shutdown)
signal.signal(signal.SIGINT, shutdown)

async def main():
    logger.info('Function registered: ' + {function_name})
    while True:
        await asyncio.sleep(1)

asyncio.run(main())
"#,
            engine_url = engine_url,
            code = params.code,
            function_name = function_name,
            description = description,
        )
    }

    async fn spawn_worker(
        &self,
        language: &str,
        temp_dir: &PathBuf,
        file_name: &str,
    ) -> Result<Child, String> {
        let cmd = match language {
            "node" | "javascript" | "js" => "node",
            "python" | "py" => "python3",
            _ => return Err(format!("Unsupported language: {}", language)),
        };

        Command::new(cmd)
            .arg(temp_dir.join(file_name))
            .current_dir(temp_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn {} process: {}", cmd, e))
    }
}
