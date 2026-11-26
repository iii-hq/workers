//! Real engine + IDE startup/deletion regression. Run with III_BIN and
//! III_IDE_BIN pointing to built binaries, then `cargo test --test
//! turn_retention -- --ignored --nocapture`. No production engine is used.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{CallRequest, Error, InitOptions, RegisterFunction, register_worker};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn require_root_ref_inventory() {
    let help = std::process::Command::new("git")
        .args(["for-each-ref", "-h"])
        .env("LC_ALL", "C")
        .output()
        .expect("turn_retention requires Git 2.45 or newer on PATH");
    // Git's help exits with status 129; check the option rather than its status.
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("include-root-refs")
            || String::from_utf8_lossy(&help.stderr).contains("include-root-refs"),
        "turn_retention requires Git 2.45 or newer with for-each-ref --include-root-refs; \
         update Git on PATH before running the real-engine test"
    );
}

fn git(repo: &Path, args: &[&str], input: Option<&str>) -> String {
    let mut child = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let out = std::thread::scope(|scope| {
        let writer = input.map(|input| {
            let mut stdin = child.stdin.take().unwrap();
            scope.spawn(move || stdin.write_all(input.as_bytes()))
        });
        let output = child.wait_with_output();
        if let Some(writer) = writer {
            writer.join().unwrap().unwrap();
        }
        output.unwrap()
    });
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn seed(store: &Path, root: &Path, session: &str, record: bool) -> PathBuf {
    let repo = store
        .join("repos")
        .join(&format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()))[..16]);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--bare"], None);
    let blob = git(&repo, &["hash-object", "-w", "--stdin"], Some(session));
    let tree = git(
        &repo,
        &["mktree"],
        Some(&format!("100644 blob {blob}\tfile.txt\n")),
    );
    git(
        &repo,
        &[
            "update-ref",
            &format!("refs/turns/{session}/turn/before"),
            &tree,
        ],
        None,
    );
    git(&repo, &["repack", "-ad"], None);
    if record {
        std::fs::create_dir_all(store.join("sessions")).unwrap();
        std::fs::write(
            store.join("sessions").join(format!("{session}.json")),
            json!({
                "session_id": session, "turns": [{"turn_id":"turn", "started_at":1,
                    "snapshot":{"root":root, "before":tree}}]
            })
            .to_string(),
        )
        .unwrap();
    }
    repo
}

fn records(store: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(store.join("sessions"))
        .unwrap()
        .map(|entry| {
            let p = entry.unwrap().path();
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                std::fs::read(p).unwrap(),
            )
        })
        .collect()
}

fn refs(store: &Path) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(store.join("repos")).unwrap() {
        let repo = entry.unwrap().path();
        for line in git(
            &repo,
            &[
                "for-each-ref",
                "--include-root-refs",
                "--format=%(refname) %(objectname)",
            ],
            None,
        )
        .lines()
        {
            let (name, oid) = line.split_once(' ').unwrap();
            result.insert(
                format!("{}/{name}", repo.file_name().unwrap().to_str().unwrap()),
                oid.to_owned(),
            );
        }
        for name in ["FETCH_HEAD", "MERGE_HEAD"] {
            if let Ok(contents) = std::fs::read_to_string(repo.join(name))
                && !contents.trim().is_empty()
            {
                result.insert(
                    format!("{}/{name}", repo.file_name().unwrap().to_str().unwrap()),
                    git(&repo, &["rev-parse", "--verify", name], None),
                );
            }
        }
    }
    result
}

fn tree_inventory(store: &Path, refs: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut grouped: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (reference, oid) in refs {
        grouped
            .entry(reference.split('/').next().unwrap())
            .or_default()
            .insert(oid);
    }
    grouped
        .into_iter()
        .map(|(repo, objects)| {
            let input = objects.into_iter().collect::<Vec<_>>().join("\n") + "\n";
            (
                repo.to_owned(),
                git(
                    &store.join("repos").join(repo),
                    &["cat-file", "--batch-check"],
                    Some(&input),
                ),
            )
        })
        .collect()
}

fn retain_symbolic_targets(store: &Path, retained: &mut BTreeSet<String>) {
    let mut targets = BTreeMap::new();
    for entry in std::fs::read_dir(store.join("repos")).unwrap() {
        let repo = entry.unwrap().path();
        let key = repo.file_name().unwrap().to_str().unwrap();
        for line in git(
            &repo,
            &[
                "for-each-ref",
                "--include-root-refs",
                "--format=%(refname) %(symref)",
            ],
            None,
        )
        .lines()
        {
            let Some((name, resolved_target)) = line.split_once(' ') else {
                continue;
            };
            if !resolved_target.is_empty() {
                // %(symref) resolves a chain; keep its intermediate refs too.
                let target = git(&repo, &["symbolic-ref", "--no-recurse", name], None);
                targets.insert(format!("{key}/{name}"), format!("{key}/{target}"));
            }
        }
    }
    let mut pending: Vec<_> = retained.iter().cloned().collect();
    while let Some(name) = pending.pop() {
        if let Some(target) = targets.get(&name)
            && retained.insert(target.clone())
        {
            pending.push(target.clone());
        }
    }
}

fn safe(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[test]
fn batch_inventory_drains_output_while_writing_more_than_a_pipe_buffer() {
    let directory = tempfile::tempdir().unwrap();
    let repo = directory.path();
    git(repo, &["init", "--bare"], None);
    let oid = git(repo, &["hash-object", "-w", "--stdin"], Some("data"));
    for count in [1, 32_768] {
        let input = format!("{oid}\n").repeat(count);
        let output = git(repo, &["cat-file", "--batch-check"], Some(&input));
        assert_eq!(output.lines().count(), count);
        assert!(output.lines().all(|line| line == format!("{oid} blob 4")));
    }
}

fn wanted_refs(records: &BTreeMap<String, Vec<u8>>, live: &BTreeSet<String>) -> BTreeSet<String> {
    let mut wanted = BTreeSet::new();
    for (filename, bytes) in records {
        let r: Value = serde_json::from_slice(bytes).unwrap();
        if !live.contains(r["session_id"].as_str().unwrap()) {
            continue;
        }
        for t in r["turns"].as_array().unwrap() {
            let Some(root) = t["snapshot"]["root"].as_str() else {
                continue;
            };
            let key = format!("{:x}", Sha256::digest(root.as_bytes()));
            for side in ["before", "after"] {
                if t["snapshot"][side].is_string() {
                    wanted.insert(format!(
                        "{}/refs/turns/{}/{}/{}",
                        &key[..16],
                        safe(filename.strip_suffix(".json").unwrap()),
                        safe(t["turn_id"].as_str().unwrap()),
                        side
                    ));
                }
            }
        }
    }
    wanted
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "worker did not reconcile the store"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn start_worker(binary: &Path, run: &Path, url: &str, name: &str) -> tokio::process::Child {
    let log = std::fs::File::create(run.join(format!("{name}.log"))).unwrap();
    tokio::process::Command::new(binary)
        .current_dir(run)
        .args(["--url", url, "--config"])
        .arg(run.join("ide.json"))
        .env("HOME", run.join("home"))
        .env("III_COMPOSE_DIR", run)
        .env("III_NAMESPACE", "default")
        .env("III_CONFIG_NAME", "retention-test-ide")
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Git 2.45+, III_BIN and a release III_IDE_BIN; launches an isolated engine"]
async fn worker_startup_and_session_deletion_reconcile_real_git_repositories() {
    require_root_ref_inventory();
    let engine = std::env::var_os("III_BIN").expect("III_BIN is required");
    let worker = PathBuf::from(std::env::var_os("III_IDE_BIN").expect("III_IDE_BIN is required"));
    let temporary = tempfile::tempdir().unwrap();
    let run = temporary.path();
    let external_copy = std::env::var_os("III_TURN_TEST_COPY").map(PathBuf::from);
    let store = external_copy.clone().unwrap_or_else(|| run.join("turns"));
    let live: BTreeSet<String> = if external_copy.is_some() {
        // Opt-in operator validation only: the marker must be created on an
        // independent copy. Never pass a running worker's store here.
        assert!(store.join(".turn-retention-test-copy").is_file());
        serde_json::from_slice(&std::fs::read(store.join(".live-sessions.json")).unwrap()).unwrap()
    } else {
        seed(&store, &run.join("missing-workspace"), "deleted", true);
        seed(&store, &run.join("missing-workspace"), "live", true);
        seed(&store, &run.join("orphan-workspace"), "orphan", false);
        let foreign = seed(&store, &run.join("foreign-workspace"), "foreign", false);
        let target = "refs/turns/foreign/turn/before";
        let tree = git(&foreign, &["rev-parse", target], None);
        git(&foreign, &["update-ref", "refs/keep/manual", &tree], None);
        git(&foreign, &["symbolic-ref", "refs/keep/alias", target], None);
        git(
            &foreign,
            &["symbolic-ref", "refs/turns/stale", "refs/keep/manual"],
            None,
        );
        let detached = seed(&store, &run.join("detached-head"), "detached", false);
        let tree = git(
            &detached,
            &["rev-parse", "refs/turns/detached/turn/before"],
            None,
        );
        std::fs::write(detached.join("HEAD"), format!("{tree}\n")).unwrap();
        BTreeSet::from(["live".to_owned()])
    };
    let before_records = records(&store);
    let before_refs = refs(&store);
    let mut wanted = wanted_refs(&before_records, &live);
    wanted.extend(
        before_refs
            .keys()
            .filter(|reference| {
                !reference
                    .split_once('/')
                    .unwrap()
                    .1
                    .starts_with("refs/turns/")
            })
            .cloned(),
    );
    retain_symbolic_targets(&store, &mut wanted);
    let expected_records: BTreeMap<_, _> = before_records
        .iter()
        .filter(|(_, bytes)| {
            let r: Value = serde_json::from_slice(bytes).unwrap();
            live.contains(r["session_id"].as_str().unwrap())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let expected_refs: BTreeMap<_, _> = before_refs
        .iter()
        .filter(|(k, _)| wanted.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let expected_repos: BTreeSet<_> = wanted
        .iter()
        .map(|r| r.split('/').next().unwrap().to_owned())
        .collect();
    let original_trees = tree_inventory(&store, &expected_refs);
    std::fs::create_dir_all(run.join("home")).unwrap();
    std::fs::write(
        run.join("ide.json"),
        json!({"turns":{"data_dir":store},"fs":{"host_roots":[run]}}).to_string(),
    )
    .unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("ws://127.0.0.1:{port}");
    std::fs::write(run.join("engine.json"), json!({"workers":[{"name":"iii-worker-manager","config":{"host":"127.0.0.1","port":port}}]}).to_string()).unwrap();
    let engine_log = std::fs::File::create(run.join("engine.log")).unwrap();
    let mut engine = tokio::process::Command::new(engine)
        .current_dir(run)
        .args(["--no-update-check", "-c"])
        .arg(run.join("engine.json"))
        .env("HOME", run.join("home"))
        .stdout(engine_log.try_clone().unwrap())
        .stderr(engine_log)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    wait_for(|| std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    let authority = register_worker(&url, InitOptions::default());
    let catalog = Arc::new(Mutex::new(live));
    let catalog_read = catalog.clone();
    authority.register_function(
        "session::get",
        RegisterFunction::new(move |input: Value| {
            let sid = input["session_id"].as_str().unwrap();
            Ok::<_, Error>(if catalog_read.lock().unwrap().contains(sid) {
                json!({"meta":{"id":sid}})
            } else {
                Value::Null
            })
        }),
    );
    authority.register_function(
        "session::on-deleted",
        RegisterFunction::new(|event: Value| Ok::<_, Error>(event)),
    );
    // Wait for the fixture functions to register before starting the IDE.
    let mut available = false;
    for _ in 0..40 {
        if authority
            .call(CallRequest {
                function_id: "session::get".into(),
                payload: json!({"session_id":"probe"}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            available = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(available);
    if let Some(baseline) = std::env::var_os("III_IDE_BASELINE") {
        let mut old = start_worker(Path::new(&baseline), run, &url, "baseline");
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(
            old.try_wait().unwrap().is_none(),
            "baseline exited: {}",
            std::fs::read_to_string(run.join("baseline.log")).unwrap()
        );
        assert!(
            records(&store) == before_records,
            "baseline changed the records"
        );
        assert!(refs(&store) == before_refs, "baseline changed the refs");
        old.kill().await.unwrap();
        old.wait().await.unwrap();
        eprintln!("baseline worker left all orphan records and refs unchanged");
    }
    let mut ide = start_worker(&worker, run, &url, "fixed");
    wait_for(|| {
        std::fs::read_to_string(run.join("fixed.log"))
            .unwrap()
            .contains("maintenance completed")
    })
    .await;
    let logs = std::fs::read_to_string(run.join("fixed.log")).unwrap();
    assert!(!logs.contains("authority_unavailable=true"), "{logs}");
    let interface: Value = authority
        .call(CallRequest {
            function_id: "engine::functions::info".into(),
            payload: json!({"function_id":"shell::turns::on-session-deleted"}),
            action: None,
            timeout_ms: Some(2000),
        })
        .await
        .unwrap();
    for field in ["request_schema", "response_schema"] {
        assert_eq!(
            interface[field]["type"], "object",
            "the registered deletion observer must expose typed schemas"
        );
    }
    // A real-store validation must not print private record contents on failure.
    assert!(
        records(&store) == expected_records,
        "retained record bytes changed"
    );
    assert!(
        refs(&store) == expected_refs,
        "retained reference targets changed"
    );
    assert!(
        tree_inventory(&store, &expected_refs) == original_trees,
        "retained Git objects changed"
    );
    let actual_repos: BTreeSet<_> = std::fs::read_dir(store.join("repos"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(actual_repos, expected_repos);
    eprintln!(
        "startup: sessions {} -> {}, refs {} -> {}, remaining repos {}",
        before_records.len(),
        expected_records.len(),
        before_refs.len(),
        expected_refs.len(),
        actual_repos.len()
    );
    if external_copy.is_none() {
        let (reference, tree) = expected_refs
            .iter()
            .find(|(name, _)| name.ends_with("/refs/turns/live/turn/before"))
            .unwrap();
        let repo_key = reference.split_once('/').unwrap().0;
        let repo = store.join("repos").join(repo_key);
        let foreign_refs: BTreeMap<_, _> = expected_refs
            .iter()
            .filter(|(name, _)| !name.starts_with(&format!("{repo_key}/")))
            .map(|(name, oid)| (name.clone(), oid.clone()))
            .collect();
        assert_eq!(
            git(&repo, &["show", &format!("{tree}:file.txt")], None),
            "live"
        );
        catalog.lock().unwrap().remove("live");
        authority
            .call(CallRequest {
                function_id: "session::on-deleted".into(),
                payload: json!({"session_id":"live"}),
                action: None,
                timeout_ms: Some(2000),
            })
            .await
            .unwrap();
        wait_for(|| !repo.exists()).await;
        assert!(records(&store).is_empty());
        assert_eq!(refs(&store), foreign_refs);
        eprintln!(
            "session deletion event reclaimed the shared repository and preserved foreign refs"
        );
    }
    ide.kill().await.unwrap();
    ide.wait().await.unwrap();
    authority.shutdown_async().await;
    engine.kill().await.unwrap();
    engine.wait().await.unwrap();
}
