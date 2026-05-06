//! Integration tests for the jobs lifecycle (`list_all`, `running_count`,
//! `remove_old`). Each test uses a unique ID prefix to avoid collisions in
//! the global `JOBS` map shared across the integration-test binary.

use iii_shell::jobs::{self, now_ms, JobHandle, JobRecord, JobStatus};

async fn seed(handle: JobHandle) -> String {
    match jobs::try_reserve_and_insert(handle, usize::MAX).await {
        Ok(id) => id,
        Err(_) => panic!("usize::MAX cap must always accept"),
    }
}

fn rec(id: &str, status: JobStatus, finished_at_ms: Option<u64>) -> JobRecord {
    JobRecord {
        id: id.to_string(),
        argv: vec!["echo".to_string(), id.to_string()],
        started_at_ms: now_ms(),
        finished_at_ms,
        status,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    }
}

#[tokio::test]
async fn now_ms_returns_a_recent_unix_ms() {
    let t = now_ms();
    assert!(t > 1_704_067_200_000, "now_ms looks pre-2024: {t}");
}

#[tokio::test]
async fn insert_then_get_round_trips_the_record() {
    let id = "lifecycle-insert-get";
    seed(JobHandle {
        record: rec(id, JobStatus::Running, None),
        child: None,
    })
    .await;
    let h = jobs::get(id).await.expect("inserted job exists");
    let got = h.lock().await.record.clone();
    assert_eq!(got.id, id);
    assert_eq!(got.status, JobStatus::Running);
    assert_eq!(got.argv, vec!["echo".to_string(), id.to_string()]);
}

#[tokio::test]
async fn get_returns_none_for_unknown_id() {
    let h = jobs::get("never-inserted-9876").await;
    assert!(h.is_none());
}

#[tokio::test]
async fn list_all_includes_inserted_jobs() {
    let id1 = "lifecycle-list-all-1";
    let id2 = "lifecycle-list-all-2";
    seed(JobHandle {
        record: rec(id1, JobStatus::Running, None),
        child: None,
    })
    .await;
    seed(JobHandle {
        record: rec(id2, JobStatus::Finished, Some(now_ms())),
        child: None,
    })
    .await;
    let all = jobs::list_all().await;
    let ids: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&id1), "list missing {id1}");
    assert!(ids.contains(&id2), "list missing {id2}");
}

#[tokio::test]
async fn running_count_excludes_terminal_states() {
    let running_id = "lifecycle-rc-running";
    let finished_id = "lifecycle-rc-finished";
    let killed_id = "lifecycle-rc-killed";
    let failed_id = "lifecycle-rc-failed";
    seed(JobHandle {
        record: rec(running_id, JobStatus::Running, None),
        child: None,
    })
    .await;
    seed(JobHandle {
        record: rec(finished_id, JobStatus::Finished, Some(now_ms())),
        child: None,
    })
    .await;
    seed(JobHandle {
        record: rec(killed_id, JobStatus::Killed, Some(now_ms())),
        child: None,
    })
    .await;
    seed(JobHandle {
        record: rec(failed_id, JobStatus::Failed, Some(now_ms())),
        child: None,
    })
    .await;

    let n = jobs::running_count().await;
    assert!(n >= 1, "running_count should include our running job");

    let all = jobs::list_all().await;
    for id in [finished_id, killed_id, failed_id] {
        let r = all
            .iter()
            .find(|r| r.id == id)
            .expect("inserted job present");
        assert_ne!(r.status, JobStatus::Running, "{id} must not be Running");
    }
}

#[tokio::test]
async fn remove_old_retention_matrix() {
    let stale_id = "lifecycle-remove-old-stale";
    let running_id = "lifecycle-remove-old-running";
    let fresh_id = "lifecycle-remove-old-fresh";

    let stale_finished = now_ms().saturating_sub(60 * 60 * 1000);
    seed(JobHandle {
        record: rec(stale_id, JobStatus::Finished, Some(stale_finished)),
        child: None,
    })
    .await;
    assert!(jobs::get(stale_id).await.is_some(), "pre-prune sanity");
    jobs::remove_old(1).await;
    assert!(
        jobs::get(stale_id).await.is_none(),
        "stale finished job should have been pruned",
    );

    seed(JobHandle {
        record: rec(running_id, JobStatus::Running, None),
        child: None,
    })
    .await;
    jobs::remove_old(0).await;
    assert!(
        jobs::get(running_id).await.is_some(),
        "running jobs (finished_at_ms = None) must survive remove_old(0)",
    );

    let recent = now_ms().saturating_sub(10);
    seed(JobHandle {
        record: rec(fresh_id, JobStatus::Finished, Some(recent)),
        child: None,
    })
    .await;
    jobs::remove_old(60).await;
    assert!(
        jobs::get(fresh_id).await.is_some(),
        "fresh job should remain within 60s retention window",
    );
}

#[tokio::test]
async fn concurrent_inserts_and_lookups_dont_deadlock() {
    let mut handles = Vec::new();
    for i in 0..50 {
        let h = tokio::spawn(async move {
            let id = format!("lifecycle-concurrent-{i}");
            seed(JobHandle {
                record: rec(&id, JobStatus::Running, None),
                child: None,
            })
            .await;
            jobs::get(&id).await.is_some()
        });
        handles.push(h);
    }
    for h in handles {
        assert!(h.await.unwrap(), "concurrent insert/get failed");
    }
    let all = jobs::list_all().await;
    let ours: usize = all
        .iter()
        .filter(|r| r.id.starts_with("lifecycle-concurrent-"))
        .count();
    assert_eq!(ours, 50, "all 50 concurrent jobs visible in list_all");
}
