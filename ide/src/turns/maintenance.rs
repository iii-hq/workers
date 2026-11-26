//! Reconcile private Git refs with durable turn records, including after downtime.
//!
//! A missing workspace does not make its history disposable. Only refs absent
//! from the records, or sessions confirmed absent by the session authority, are
//! removed. No Git operation here needs to open the original workspace.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::process::Stdio;
use std::time::Duration;

use super::*;

const DELETED_FN_ID: &str = "shell::turns::on-session-deleted";
const INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize, JsonSchema)]
struct SessionDeleted {
    session_id: String,
}

#[derive(Debug, Default, Serialize)]
struct Summary {
    removed_sessions: usize,
    removed_refs: usize,
    removed_repos: usize,
    authority_unavailable: bool,
}

struct StoredSession {
    path: PathBuf,
    bytes: Vec<u8>,
    record: SessionRecord,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn repo_key(root: &str) -> String {
    format!("{:x}", Sha256::digest(root.as_bytes()))[..16].to_owned()
}

/// Refuse symlinks: reconciliation must never mutate another store through one.
async fn entries(path: &Path) -> io::Result<Vec<tokio::fs::DirEntry>> {
    match tokio::fs::symlink_metadata(path).await {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
        Ok(meta) if !meta.is_dir() => {
            return Err(invalid("turn store directory is not a directory"));
        }
        Ok(_) => {}
    }
    let mut dir = tokio::fs::read_dir(path).await?;
    let mut result = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        result.push(entry);
    }
    Ok(result)
}

async fn sessions(store: &Path) -> io::Result<Vec<StoredSession>> {
    let mut result = Vec::new();
    for entry in entries(&store.join("sessions")).await? {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        if !entry.file_type().await?.is_file() {
            return Err(invalid("turn session record is not a regular file"));
        }
        let bytes = tokio::fs::read(&path).await?;
        // Unlike an interactive load, a corrupt record must abort cleanup:
        // treating it as an empty session would discard its snapshots.
        let record: SessionRecord = serde_json::from_slice(&bytes).map_err(invalid_json)?;
        if record.session_id.is_empty()
            || path.file_stem().and_then(|s| s.to_str())
                != Some(session_file_stem(&record.session_id).as_str())
        {
            return Err(invalid("turn session record has an inconsistent identity"));
        }
        result.push(StoredSession {
            path,
            bytes,
            record,
        });
    }
    Ok(result)
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

async fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(repo)
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "snapshot Git operation timed out")
        })??;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "snapshot Git operation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(|_| invalid("non-UTF8 snapshot Git output"))
}

async fn private_repo(repo: &Path) -> io::Result<()> {
    if !tokio::fs::symlink_metadata(repo).await?.is_dir() {
        return Err(invalid("snapshot repository is not a directory"));
    }
    for name in ["HEAD", "config", "objects", "refs", "packed-refs"] {
        match tokio::fs::symlink_metadata(repo.join(name)).await {
            Ok(meta) if meta.is_symlink() => {
                return Err(invalid("snapshot repository contains a symlink"));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound && name == "packed-refs" => {}
            Err(e) => return Err(e),
            _ => {}
        }
    }
    if git(repo, &["rev-parse", "--is-bare-repository"])
        .await?
        .trim()
        != "true"
    {
        return Err(invalid("snapshot repository is not bare"));
    }
    Ok(())
}

pub(super) async fn drop_turn_refs(
    repos: &Path,
    root: &str,
    session: &str,
    turn: &str,
) -> io::Result<()> {
    let repo = repos.join(repo_key(root));
    if !tokio::fs::try_exists(&repo).await? {
        return Ok(());
    }
    private_repo(&repo).await?;
    for side in [Side::Before, Side::After] {
        git(
            &repo,
            &[
                "update-ref",
                "-d",
                &ref_name(&session_file_stem(session), turn, side),
            ],
        )
        .await?;
    }
    Ok(())
}

impl TurnLog {
    async fn reconcile_snapshots<F, Fut>(&self, exists: F) -> io::Result<Summary>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<bool, Error>>,
    {
        let mut summary = Summary::default();
        let mut missing = BTreeMap::new();
        // Dependency calls are outside the write guard so an unavailable
        // session worker never stalls active turns. Recheck bytes under it.
        for session in sessions(&self.store.dir).await? {
            match exists(session.record.session_id.clone()).await {
                Ok(false) => {
                    missing.insert(session.path, session.bytes);
                }
                Ok(true) => {}
                Err(_) => {
                    summary.authority_unavailable = true;
                    break;
                }
            }
        }
        let _guard = self.snapshot_gate.write().await;
        let current = sessions(&self.store.dir).await?;
        let mut expected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for session in current {
            if missing.get(&session.path) == Some(&session.bytes) {
                tokio::fs::remove_file(&session.path).await?;
                summary.removed_sessions += 1;
                continue;
            }
            let stem = session_file_stem(&session.record.session_id);
            for turn in session.record.turns {
                let Some(snapshot) = turn.snapshot else {
                    continue;
                };
                let refs = expected.entry(repo_key(&snapshot.root)).or_default();
                for (side, tree) in [
                    (Side::Before, snapshot.before),
                    (Side::After, snapshot.after),
                ] {
                    if tree.is_some() {
                        refs.insert(ref_name(&stem, &turn.turn_id, side));
                    }
                }
            }
        }
        for entry in entries(&self.repos_dir()).await? {
            let key = entry.file_name();
            let Some(key) = key
                .to_str()
                .filter(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            else {
                continue;
            };
            let repo = entry.path();
            private_repo(&repo).await?;
            let refs = git(
                &repo,
                &["for-each-ref", "--format=%(refname) %(objectname)"],
            )
            .await?;
            let mut foreign = false;
            for line in refs.lines() {
                let Some((name, oid)) = line.split_once(' ') else {
                    return Err(invalid("invalid snapshot ref listing"));
                };
                if !name.starts_with("refs/turns/") {
                    foreign = true;
                } else if !expected
                    .get(key)
                    .is_some_and(|wanted| wanted.contains(name))
                {
                    git(&repo, &["update-ref", "-d", name, oid]).await?;
                    summary.removed_refs += 1;
                }
            }
            if !foreign && expected.get(key).is_none_or(BTreeSet::is_empty) {
                // The record scan and all writers share snapshot_gate. The
                // repository has no retained snapshots, so packs and abandoned
                // indexes can go together without a costly repack.
                tokio::fs::remove_dir_all(&repo).await?;
                summary.removed_repos += 1;
            }
        }
        Ok(summary)
    }
}

pub(super) fn register(iii: &IIIClient, log: Arc<TurnLog>) {
    let wake = Arc::new(tokio::sync::Notify::new());
    let notified = wake.clone();
    iii.register_function(
        DELETED_FN_ID,
        RegisterFunction::new_async(move |event: Delivered<SessionDeleted>| {
            let notified = notified.clone();
            async move {
                // Recheck the authority instead of trusting a delivered payload.
                if delivered_event(event).is_some_and(|event| !event.session_id.is_empty()) {
                    notified.notify_one();
                }
                Ok::<Ack, Error>(Ack { ok: true })
            }
        })
        .description("Internal: reconcile turn snapshots after a session is deleted.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    iii.observe(
        Source::function("session::on-deleted").capture([Capture::Output]),
        DELETED_FN_ID,
    );
    let iii = iii.clone();
    tokio::spawn(async move {
        loop {
            let result = log
                .reconcile_snapshots(|session_id| {
                    let iii = iii.clone();
                    async move {
                        let response = iii
                            .call(iii_sdk::CallRequest {
                                function_id: "session::get".into(),
                                payload: json!({ "session_id": session_id }),
                                action: None,
                                timeout_ms: Some(2_000),
                            })
                            .await?;
                        if response.is_null() {
                            return Ok(false);
                        }
                        if response.get("meta").is_some_and(Value::is_object) {
                            return Ok(true);
                        }
                        Err(Error::Handler("invalid session authority response".into()))
                    }
                })
                .await;
            let retry_delay = match &result {
                Ok(summary) if summary.authority_unavailable => Duration::from_secs(30),
                Err(_) => Duration::from_secs(30),
                _ => INTERVAL,
            };
            match result {
                Ok(summary) => tracing::info!(
                    removed_sessions = summary.removed_sessions,
                    removed_refs = summary.removed_refs,
                    removed_repos = summary.removed_repos,
                    authority_unavailable = summary.authority_unavailable,
                    "turn snapshots: maintenance completed"
                ),
                Err(error) => tracing::warn!(%error, "turn snapshots: maintenance deferred"),
            }
            tokio::select! {
                () = wake.notified() => {}
                () = tokio::time::sleep(retry_delay) => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn snapshot(log: &TurnLog, root: &Path, session: &str) -> (PathBuf, String) {
        tokio::fs::create_dir_all(root).await.unwrap();
        tokio::fs::write(root.join("file.txt"), session)
            .await
            .unwrap();
        log.snapshot_turn(
            &RootTurn {
                session_id: session.into(),
                turn_id: "turn".into(),
            },
            root.to_str().unwrap(),
            Side::Before,
        )
        .await;
        let record = log.store.load(session).await.unwrap();
        let snapshot = record.turns[0].snapshot.as_ref().unwrap();
        (
            log.repos_dir().join(repo_key(&snapshot.root)),
            snapshot.before.clone().unwrap(),
        )
    }

    fn log(dir: &Path) -> Arc<TurnLog> {
        Arc::new(TurnLog::new(
            TurnStore::new(dir.join("store"), 1024).with_snapshot_min_free_bytes(0),
        ))
    }

    #[tokio::test]
    async fn startup_reclaims_a_packed_orphan_repository_without_its_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let root = dir.path().join("workspace");
        let (repo, _) = snapshot(&log, &root, "deleted").await;
        git(&repo, &["repack", "-ad"]).await.unwrap();
        assert!(
            tokio::fs::read_dir(repo.join("objects/pack"))
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_some()
        );
        tokio::fs::remove_file(log.store.session_path("deleted"))
            .await
            .unwrap();
        tokio::fs::remove_dir_all(&root).await.unwrap();
        let result = log
            .reconcile_snapshots(|_| async { Ok(true) })
            .await
            .unwrap();
        assert_eq!(result.removed_repos, 1);
        assert_eq!(result.removed_refs, 1);
        assert!(!repo.exists());
    }

    #[tokio::test]
    async fn confirmed_deleted_session_is_removed_but_shared_live_history_is_readable() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let root = dir.path().join("workspace");
        let (repo, _) = snapshot(&log, &root, "deleted").await;
        let (_, live_tree) = snapshot(&log, &root, "live").await;
        // Workspace disappearance alone must never evict retained history.
        tokio::fs::remove_dir_all(&root).await.unwrap();
        let result = log
            .reconcile_snapshots(|sid| async move { Ok(sid != "deleted") })
            .await
            .unwrap();
        assert_eq!(result.removed_sessions, 1);
        assert_eq!(result.removed_refs, 1);
        assert_eq!(result.removed_repos, 0);
        assert!(!log.store.session_path("deleted").exists());
        assert!(log.store.session_path("live").exists());
        let refs = git(&repo, &["for-each-ref", "--format=%(refname)"])
            .await
            .unwrap();
        assert_eq!(refs.trim(), "refs/turns/live/turn/before");
        assert_eq!(
            git(&repo, &["show", &format!("{live_tree}:file.txt")])
                .await
                .unwrap(),
            "live"
        );
    }

    #[tokio::test]
    async fn absent_session_authority_preserves_records_and_their_refs() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let (repo, _) = snapshot(&log, &dir.path().join("workspace"), "live").await;
        let result = log
            .reconcile_snapshots(|_| async { Err(Error::Timeout) })
            .await
            .unwrap();
        assert!(result.authority_unavailable);
        assert_eq!(result.removed_sessions, 0);
        assert_eq!(result.removed_refs, 0);
        assert!(repo.exists());
    }

    #[tokio::test]
    async fn corrupt_session_record_aborts_cleanup_without_discarding_its_refs() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let (repo, _) = snapshot(&log, &dir.path().join("workspace"), "live").await;
        tokio::fs::write(log.store.session_path("live"), b"{broken")
            .await
            .unwrap();
        assert!(
            log.reconcile_snapshots(|_| async { Ok(false) })
                .await
                .is_err()
        );
        assert!(
            git(&repo, &["for-each-ref", "refs/turns"])
                .await
                .unwrap()
                .contains("/live/")
        );
    }

    #[tokio::test]
    async fn changed_record_is_rechecked_after_authority_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let (repo, _) = snapshot(&log, &dir.path().join("workspace"), "live").await;
        let result = log
            .reconcile_snapshots(|sid| {
                let log = log.clone();
                async move {
                    log.on_turn_started(&sid, "new-turn").await.unwrap();
                    Ok(false)
                }
            })
            .await
            .unwrap();
        assert_eq!(result.removed_sessions, 0);
        assert_eq!(log.store.load("live").await.unwrap().turns.len(), 2);
        assert!(repo.exists());
    }

    #[tokio::test]
    async fn reconciliation_waits_for_the_snapshot_record_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let (repo, _) = snapshot(&log, &dir.path().join("workspace"), "live").await;
        let guard = log.snapshot_gate.read().await;
        let queried = Arc::new(tokio::sync::Notify::new());
        let observed = queried.clone();
        let cloned = log.clone();
        let task = tokio::spawn(async move {
            cloned
                .reconcile_snapshots(|_| {
                    observed.notify_one();
                    async { Ok(true) }
                })
                .await
        });
        queried.notified().await;
        assert!(!task.is_finished());
        drop(guard);
        assert_eq!(task.await.unwrap().unwrap().removed_refs, 0);
        assert!(repo.exists());
    }

    #[tokio::test]
    async fn foreign_refs_keep_an_otherwise_unreferenced_repository() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let (repo, tree) = snapshot(&log, &dir.path().join("workspace"), "deleted").await;
        git(&repo, &["update-ref", "refs/keep/manual", &tree])
            .await
            .unwrap();
        let result = log
            .reconcile_snapshots(|_| async { Ok(false) })
            .await
            .unwrap();
        assert_eq!(result.removed_refs, 1);
        assert_eq!(result.removed_repos, 0);
        assert!(
            git(&repo, &["for-each-ref"])
                .await
                .unwrap()
                .contains("refs/keep/manual")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_repository_is_never_followed_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let (repo, _) = snapshot(&log, &dir.path().join("workspace"), "live").await;
        let elsewhere = dir.path().join("elsewhere");
        tokio::fs::rename(&repo, &elsewhere).await.unwrap();
        std::os::unix::fs::symlink(&elsewhere, &repo).unwrap();
        assert!(
            log.reconcile_snapshots(|_| async { Ok(true) })
                .await
                .is_err()
        );
        assert!(
            git(&elsewhere, &["for-each-ref"])
                .await
                .unwrap()
                .contains("/live/")
        );
        assert!(repo.is_symlink());
    }
}
