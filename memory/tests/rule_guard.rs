//! `memory::rule::set` with `expected`: a write made from a stale read is
//! refused with `rule_changed` and leaves the rule as it was.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iii_sdk::IIIClient;
use memory::config::WorkerConfig;
use memory::deps::Deps;
use memory::error::MemoryError;
use memory::functions::rules::{self, RULE_CHANGED, RuleSetResponse};
use memory::store::Store;
use serde_json::json;

const BANK: &str = "offline";
const RULE: &str = "notes";

/// Deps over a store in `dir` and no engine.
fn offline_deps(dir: &Path) -> Deps {
    let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
    Deps {
        hook_binding: Arc::default(),
        emitter: Arc::new(memory::events::Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(
            Store::open(dir.to_path_buf()).unwrap(),
        ))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
            embeddings_enabled: false,
            ..WorkerConfig::default()
        }))),
    }
}

/// The rule as `memory::rule::list` returns it; `""` when missing.
async fn read(deps: &Deps, bank: &str) -> String {
    let req = serde_json::from_value(json!({ "bank": bank })).unwrap();
    rules::list(deps, req)
        .await
        .unwrap()
        .rules
        .into_iter()
        .find(|r| r.name == RULE)
        .map(|r| r.content)
        .unwrap_or_default()
}

/// A `memory::rule::set` call as it arrives over the bus, engine-injected
/// key included.
async fn set(
    deps: &Deps,
    bank: &str,
    content: &str,
    expected: Option<&str>,
) -> Result<RuleSetResponse, MemoryError> {
    let mut req = json!({
        "bank": bank,
        "name": RULE,
        "content": content,
        "_caller_worker_id": "test",
    });
    if let Some(expected) = expected {
        req["expected"] = json!(expected);
    }
    rules::set(deps, serde_json::from_value(req).unwrap()).await
}

fn rule_file(dir: &Path, bank: &str) -> PathBuf {
    dir.join(bank).join("rules").join(format!("{RULE}.md"))
}

fn assert_rule_changed(result: Result<RuleSetResponse, MemoryError>) {
    match result {
        Err(e) => assert!(e.to_string().contains(RULE_CHANGED), "{e}"),
        Ok(r) => panic!("a stale write landed: {r:?}"),
    }
}

/// Proves the guard, not a retry policy: the retry belongs to each caller
/// (the two production callers stop at 3 tries), so every task here keeps
/// re-reading until its append lands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_concurrent_appends_to_one_rule_all_survive() {
    let dir = tempfile::tempdir().unwrap();
    let deps = Arc::new(offline_deps(dir.path()));
    set(&deps, BANK, "# notes\n", None).await.unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let (deps, barrier) = (deps.clone(), barrier.clone());
            tokio::spawn(async move {
                let line = format!("- note {i}\n");
                barrier.wait().await;
                // Each refusal means another append landed after this read,
                // so 7 others allow at most 7 refusals.
                for _ in 0..8 {
                    let current = read(&deps, BANK).await;
                    match set(&deps, BANK, &format!("{current}{line}"), Some(&current)).await {
                        Ok(_) => return,
                        Err(e) if e.to_string().contains(RULE_CHANGED) => {
                            tokio::task::yield_now().await
                        }
                        Err(e) => panic!("append {i}: {e}"),
                    }
                }
                panic!("append {i} refused without another append landing");
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    let rule = std::fs::read_to_string(rule_file(dir.path(), BANK)).unwrap();
    let mut lines: Vec<&str> = rule.lines().collect();
    lines.sort_unstable();
    let mut want = vec!["# notes".to_string()];
    want.extend((0..8).map(|i| format!("- note {i}")));
    want.sort_unstable();
    assert_eq!(lines, want, "{rule}");
}

#[tokio::test]
async fn a_stale_expected_is_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline_deps(dir.path());
    set(&deps, BANK, "- a\n", None).await.unwrap();
    let path = rule_file(dir.path(), BANK);
    let before = std::fs::read(&path).unwrap();

    assert_rule_changed(set(&deps, BANK, "- b\n", Some("- stale\n")).await);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // A missing bank holds no rule, so a non-empty `expected` is stale and
    // the bank is not created.
    assert_rule_changed(set(&deps, "ghost", "- b\n", Some("- a\n")).await);
    assert!(deps.store().await.bank("ghost").await.is_err());
    assert!(!dir.path().join("ghost").exists());

    // The current content is accepted.
    set(&deps, BANK, "- a\n- b\n", Some("- a\n")).await.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- a\n- b\n");
}

#[tokio::test]
async fn an_empty_expected_only_creates() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline_deps(dir.path());
    let created = set(&deps, BANK, "- a\n", Some("")).await.unwrap();
    assert!(created.ok && created.exists);
    let path = rule_file(dir.path(), BANK);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- a\n");

    assert_rule_changed(set(&deps, BANK, "- b\n", Some("")).await);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- a\n");
}

#[tokio::test]
async fn an_empty_content_delete_honours_expected() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline_deps(dir.path());
    set(&deps, BANK, "- a\n", None).await.unwrap();
    let path = rule_file(dir.path(), BANK);

    assert_rule_changed(set(&deps, BANK, "", Some("- stale\n")).await);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- a\n");

    let removed = set(&deps, BANK, "", Some("- a\n")).await.unwrap();
    assert!(removed.ok && !removed.exists);
    assert!(!path.exists());

    // Against a missing bank: `""` is a no-op, anything else is stale, and
    // neither creates the bank.
    set(&deps, "ghost", "", Some("")).await.unwrap();
    assert_rule_changed(set(&deps, "ghost", "", Some("- a\n")).await);
    assert!(deps.store().await.bank("ghost").await.is_err());
    assert!(!dir.path().join("ghost").exists());
}

#[tokio::test]
async fn expected_compares_redacted_text() {
    let dir = tempfile::tempdir().unwrap();
    let deps = offline_deps(dir.path());
    deps.store().await.ensure_bank(BANK, None).await.unwrap();
    // Built at runtime: no secret-shaped literal in the source.
    let password = ["Pw9", "xYz", "77"].concat();
    let secret = format!("{}://admin:{password}@db/app", "postgres");
    let path = rule_file(dir.path(), BANK);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("- connect with {secret}\n")).unwrap();

    let listed = read(&deps, BANK).await;
    assert!(!listed.contains(&password), "{listed}");
    assert!(listed.contains("[REDACTED:url_credentials]"), "{listed}");

    let next = format!("{listed}- and say hello\n");
    set(&deps, BANK, &next, Some(&listed)).await.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), next);
}
