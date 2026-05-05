use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Child;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Running,
    Finished,
    Killed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

/// Atomically count running jobs and insert a new one if under `max`. Holds
/// the map mutex across both operations so two concurrent callers cannot
/// each pass the count check and then both insert. A handle whose lock is
/// currently held by another task is conservatively counted as running —
/// it's mid-finalization and either still Running or about to be — so the
/// soft cap may briefly under-allow at the boundary, but never over-allows.
///
/// On rejection, returns the running count and the original handle so the
/// caller can reclaim the spawned child process and kill it.
pub async fn try_reserve_and_insert(
    handle: JobHandle,
    max: usize,
) -> Result<String, (usize, JobHandle)> {
    let mut guard = JOBS.map.lock().await;
    let mut running = 0usize;
    for h in guard.values() {
        match h.try_lock() {
            Ok(g) => {
                if g.record.status == JobStatus::Running {
                    running += 1;
                }
            }
            Err(_) => running += 1,
        }
    }
    if running >= max {
        return Err((running, handle));
    }
    let id = handle.record.id.clone();
    let boxed = Arc::new(Mutex::new(handle));
    guard.insert(id.clone(), boxed);
    Ok(id)
}

pub async fn get(id: &str) -> Option<Arc<Mutex<JobHandle>>> {
    JOBS.map.lock().await.get(id).cloned()
}

// Snapshot the map before awaiting per-job locks. Holding the map guard
// across `handle.lock().await` head-of-line-blocks every other job
// operation (insert, get) for the duration of the iteration.
async fn snapshot() -> Vec<(String, Arc<Mutex<JobHandle>>)> {
    let guard = JOBS.map.lock().await;
    guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

pub async fn remove_old(retention_secs: u64) {
    let now = now_ms();
    let threshold_ms = retention_secs.saturating_mul(1000);
    let handles = snapshot().await;
    let mut to_remove: Vec<String> = Vec::new();
    for (id, handle) in handles {
        let h = handle.lock().await;
        if let Some(fin) = h.record.finished_at_ms {
            if now.saturating_sub(fin) > threshold_ms {
                to_remove.push(id);
            }
        }
    }
    if !to_remove.is_empty() {
        let mut guard = JOBS.map.lock().await;
        for id in to_remove {
            guard.remove(&id);
        }
    }
}

pub async fn list_all() -> Vec<JobRecord> {
    let handles = snapshot().await;
    let mut out = Vec::with_capacity(handles.len());
    for (_, handle) in handles {
        out.push(handle.lock().await.record.clone());
    }
    out
}

/// Used by integration tests to assert lifecycle invariants. The
/// production exec_bg path uses `try_reserve_and_insert` which does the
/// counting and insertion atomically.
#[allow(dead_code)]
pub async fn running_count() -> usize {
    let handles = snapshot().await;
    let mut n = 0;
    for (_, handle) in handles {
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
        let id = format!("test-insert-{}", uuid::Uuid::new_v4());
        match try_reserve_and_insert(make_handle(&id, JobStatus::Running), usize::MAX).await {
            Ok(_) => {}
            Err(_) => panic!("usize::MAX cap must always succeed"),
        }
        let got = get(&id).await.expect("job exists");
        assert_eq!(got.lock().await.record.id, id);
        JOBS.map.lock().await.remove(&id);
    }

    fn make_handle(id: &str, status: JobStatus) -> JobHandle {
        JobHandle {
            record: JobRecord {
                id: id.into(),
                argv: vec!["x".into()],
                started_at_ms: now_ms(),
                finished_at_ms: if status == JobStatus::Running {
                    None
                } else {
                    Some(now_ms())
                },
                status,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
            child: None,
        }
    }

    // The `JOBS` singleton is shared across all tests in the binary and
    // tokio runs them concurrently, so these tests use deterministic
    // assertions that don't depend on absolute counts.

    #[tokio::test]
    async fn try_reserve_with_zero_cap_rejects_and_returns_handle() {
        let id = format!("reserve-zero-{}", uuid::Uuid::new_v4());
        match try_reserve_and_insert(make_handle(&id, JobStatus::Running), 0).await {
            Ok(_) => panic!("cap=0 must reject"),
            Err((_, returned)) => {
                assert_eq!(returned.record.id, id);
            }
        }
        assert!(
            get(&id).await.is_none(),
            "rejected reservation must not insert into the map"
        );
    }
}
