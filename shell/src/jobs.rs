use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Child;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Running,
    Finished,
    Killed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub argv: Vec<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub struct JobHandle {
    pub record: JobRecord,
    pub child: Option<Child>,
}

pub struct Jobs {
    pub map: Mutex<HashMap<String, Arc<Mutex<JobHandle>>>>,
}

impl Jobs {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }
}

pub static JOBS: Lazy<Jobs> = Lazy::new(Jobs::new);

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub async fn insert(handle: JobHandle) -> String {
    let id = handle.record.id.clone();
    let boxed = Arc::new(Mutex::new(handle));
    JOBS.map.lock().await.insert(id.clone(), boxed);
    id
}

pub async fn get(id: &str) -> Option<Arc<Mutex<JobHandle>>> {
    JOBS.map.lock().await.get(id).cloned()
}

pub async fn remove_old(retention_secs: u64) {
    let now = now_ms();
    let threshold_ms = retention_secs.saturating_mul(1000);
    let mut guard = JOBS.map.lock().await;
    let to_remove: Vec<String> = {
        let mut out = Vec::new();
        for (id, handle) in guard.iter() {
            let h = handle.lock().await;
            if let Some(fin) = h.record.finished_at_ms {
                if now.saturating_sub(fin) > threshold_ms {
                    out.push(id.clone());
                }
            }
        }
        out
    };
    for id in to_remove {
        guard.remove(&id);
    }
}

pub async fn list_all() -> Vec<JobRecord> {
    let guard = JOBS.map.lock().await;
    let mut out = Vec::with_capacity(guard.len());
    for handle in guard.values() {
        out.push(handle.lock().await.record.clone());
    }
    out
}

pub async fn running_count() -> usize {
    let guard = JOBS.map.lock().await;
    let mut n = 0;
    for handle in guard.values() {
        if handle.lock().await.record.status == JobStatus::Running {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_insert_and_get() {
        let rec = JobRecord {
            id: "test-1".to_string(),
            argv: vec!["echo".to_string()],
            started_at_ms: now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };
        insert(JobHandle {
            record: rec.clone(),
            child: None,
        })
        .await;
        let got = get("test-1").await.expect("job exists");
        assert_eq!(got.lock().await.record.id, "test-1");
    }
}
