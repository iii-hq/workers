//! Shared test harness: the hand-rolled golden-file comparer (deliberately
//! no snapshot dependency), real-temp-repo git helpers, and in-process fakes
//! for the store, emitter, enqueuer, and test gate.
//!
//! Golden workflow:
//! - `cargo test` compares actual output against the committed goldens.
//! - `UPDATE_GOLDENS=1 cargo test` regenerates the files; review the git
//!   diff, then commit the new goldens alongside the change that caused them.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use worktree::config::WorkerConfig;
use worktree::error::{codes, WError};
use worktree::events::{Emitter, EventDeliverer, EventKind, TriggerSets};
use worktree::functions::Deps;
use worktree::land::test_gate::{TestOutcome, TestRunner};
use worktree::land::Enqueuer;
use worktree::locks::RepoLocks;
use worktree::state::{InMemoryStateStore, StateStore};

// ---------------------------------------------------------------------------
// Golden-file harness
// ---------------------------------------------------------------------------

/// Root of the committed golden files.
pub fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn update_mode() -> bool {
    std::env::var("UPDATE_GOLDENS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Compare `actual` against the golden file at `tests/golden/<rel>`.
/// Returns `Err(readable diff hint)` on mismatch or missing golden;
/// with `UPDATE_GOLDENS=1` the file is (re)written and the check passes.
pub fn check_golden(rel: &str, actual: &str) -> Result<(), String> {
    let path = golden_root().join(rel);
    if update_mode() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        fs::write(&path, actual).map_err(|e| format!("write {}: {e}", path.display()))?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path).map_err(|e| {
        format!(
            "golden file {} unreadable ({e}).\n\
             Run `UPDATE_GOLDENS=1 cargo test` to (re)generate, then review \
             and commit the diff.",
            path.display()
        )
    })?;
    if expected == actual {
        return Ok(());
    }
    Err(diff_hint(rel, &expected, actual))
}

/// Readable first-divergence diff hint.
fn diff_hint(rel: &str, expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let first_diff = exp_lines
        .iter()
        .zip(act_lines.iter())
        .position(|(e, a)| e != a)
        .unwrap_or_else(|| exp_lines.len().min(act_lines.len()));

    const CONTEXT: usize = 3;
    let lo = first_diff.saturating_sub(CONTEXT);
    let hi = (first_diff + CONTEXT + 1).max(first_diff + 1);

    let mut out = format!(
        "golden mismatch: tests/golden/{rel}\n\
         first divergence at line {} (expected {} lines, actual {} lines)\n",
        first_diff + 1,
        exp_lines.len(),
        act_lines.len()
    );
    out.push_str("--- expected (golden) ---\n");
    for (i, line) in exp_lines.iter().enumerate().skip(lo).take(hi - lo) {
        let marker = if i == first_diff { ">" } else { " " };
        out.push_str(&format!("{marker} {:>4} | {line}\n", i + 1));
    }
    out.push_str("--- actual ---\n");
    for (i, line) in act_lines.iter().enumerate().skip(lo).take(hi - lo) {
        let marker = if i == first_diff { ">" } else { " " };
        out.push_str(&format!("{marker} {:>4} | {line}\n", i + 1));
    }
    out.push_str(
        "If this change is intentional, run `UPDATE_GOLDENS=1 cargo test`, \
         review the git diff, and commit the updated goldens.\n",
    );
    out
}

/// Assert a schemars-derived request/response schema is a *real* schema and
/// not the permissive `AnyValue` schema a `Value` handler emits.
pub fn assert_typed_schema(label: &str, schema: &schemars::schema::RootSchema) {
    let value = serde_json::to_value(schema).expect("schema serializes");
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{label}: schema is not a JSON object"));
    const DEFINING: [&str; 8] = [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
        "items",
    ];
    let has_defining = DEFINING.iter().any(|k| obj.contains_key(*k));
    assert!(
        has_defining,
        "{label}: schema is the permissive AnyValue/empty schema (no type/properties/$ref/…). \
         The handler is registered with `Value` — give it a typed struct deriving JsonSchema. \
         Got: {value}"
    );
}

// ---------------------------------------------------------------------------
// Real temp-repo git helpers
// ---------------------------------------------------------------------------

/// Run `git <args>` in `dir`, panicking on failure. Test setup only.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Initialize a repo on branch `main` with one commit.
pub fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    commit_file(dir, "README.md", "hello\n", "initial commit");
}

pub fn commit_file(dir: &Path, file: &str, content: &str, message: &str) -> String {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-m", message]);
    head_sha(dir)
}

pub fn head_sha(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

pub fn branch_sha(dir: &Path, branch: &str) -> String {
    git(dir, &["rev-parse", branch])
}

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

/// Records every event delivery for assertions.
#[derive(Default)]
pub struct RecordingDeliverer {
    seen: Mutex<Vec<(String, Value)>>,
}

impl RecordingDeliverer {
    pub fn events_of(&self, trigger_type: &str) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == trigger_type)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait]
impl EventDeliverer for RecordingDeliverer {
    async fn deliver(&self, trigger_type: &str, _function_id: &str, payload: Value) {
        self.seen
            .lock()
            .unwrap()
            .push((trigger_type.to_string(), payload));
    }
}

/// An emitter with a catch-all binding on every trigger type, so every
/// emitted event lands in the recorder.
pub fn recording_emitter() -> (Arc<Emitter>, Arc<RecordingDeliverer>) {
    let sets = TriggerSets::new();
    for kind in EventKind::all() {
        sets.for_kind(kind)
            .add(iii_sdk::trigger::TriggerConfig {
                id: format!("test-{}", kind.trigger_type()),
                function_id: "test::recv".into(),
                config: json!({}),
                metadata: None,
            })
            .unwrap();
    }
    let deliverer = Arc::new(RecordingDeliverer::default());
    (Arc::new(Emitter::new(sets, deliverer.clone())), deliverer)
}

/// Records enqueue calls; never dispatches.
#[derive(Default)]
pub struct FakeEnqueuer {
    pub calls: Mutex<Vec<(String, String, Value)>>,
}

impl FakeEnqueuer {
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn last(&self) -> Option<(String, String, Value)> {
        self.calls.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl Enqueuer for FakeEnqueuer {
    async fn enqueue(&self, queue: &str, function_id: &str, payload: Value) -> Result<(), WError> {
        self.calls
            .lock()
            .unwrap()
            .push((queue.to_string(), function_id.to_string(), payload));
        Ok(())
    }
}

/// A test gate scripted per call: `script(call_index) -> exit_code`. The
/// closure may mutate the repo (e.g. advance the target branch) to simulate
/// races.
pub struct ScriptedTestRunner {
    calls: AtomicU32,
    script: Box<dyn Fn(u32) -> i32 + Send + Sync>,
}

impl ScriptedTestRunner {
    pub fn new(script: impl Fn(u32) -> i32 + Send + Sync + 'static) -> Self {
        Self {
            calls: AtomicU32::new(0),
            script: Box::new(script),
        }
    }

    pub fn passing() -> Self {
        Self::new(|_| 0)
    }

    pub fn call_count(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TestRunner for ScriptedTestRunner {
    async fn run(&self, _cmd: &str, _wt: &str, _timeout_ms: u64) -> Result<TestOutcome, WError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let exit_code = (self.script)(n);
        Ok(TestOutcome {
            exit_code,
            tail: String::new(),
        })
    }
}

/// Store wrapper that fails the Nth `set` once (simulated crash), then
/// heals. `fail_at = 0` never fails; `set_count` exposes total writes.
pub struct FailingStore {
    inner: Arc<dyn StateStore>,
    set_calls: AtomicU32,
    fail_at: u32,
    tripped: AtomicBool,
}

impl FailingStore {
    pub fn new(inner: Arc<dyn StateStore>, fail_at: u32) -> Self {
        Self {
            inner,
            set_calls: AtomicU32::new(0),
            fail_at,
            tripped: AtomicBool::new(false),
        }
    }

    pub fn set_count(&self) -> u32 {
        self.set_calls.load(Ordering::SeqCst)
    }

    pub fn tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl StateStore for FailingStore {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<Value>, WError> {
        self.inner.get(scope, key).await
    }

    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), WError> {
        let n = self.set_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_at != 0 && n == self.fail_at && !self.tripped.swap(true, Ordering::SeqCst) {
            return Err(WError::new(
                codes::STATE_UNAVAILABLE,
                "injected crash before write",
            ));
        }
        self.inner.set(scope, key, value).await
    }

    async fn delete(&self, scope: &str, key: &str) -> Result<(), WError> {
        self.inner.delete(scope, key).await
    }

    async fn list(&self, scope: &str) -> Result<Vec<Value>, WError> {
        self.inner.list(scope).await
    }
}

// ---------------------------------------------------------------------------
// Environment builder
// ---------------------------------------------------------------------------

pub struct TestEnv {
    pub deps: Arc<Deps>,
    pub state: Arc<InMemoryStateStore>,
    pub deliverer: Arc<RecordingDeliverer>,
    pub enqueuer: Arc<FakeEnqueuer>,
}

pub fn test_config(root: &Path) -> WorkerConfig {
    let mut cfg = WorkerConfig {
        worktree_root: root.join("wts").to_string_lossy().into_owned(),
        git_timeout_ms: 30_000,
        test_timeout_ms: 30_000,
        ..WorkerConfig::default()
    };
    // Behavioral tests exercise the force paths; the gates suite covers the
    // restrictive defaults explicitly.
    cfg.gates.allow_force = true;
    cfg
}

/// In-process environment against a tempdir: in-memory store, recording
/// emitter, fake enqueuer, passing test gate.
pub fn make_env(root: &Path, cfg: WorkerConfig) -> TestEnv {
    make_env_with_runner(root, cfg, Arc::new(ScriptedTestRunner::passing()))
}

pub fn make_env_with_runner(
    _root: &Path,
    cfg: WorkerConfig,
    runner: Arc<dyn TestRunner>,
) -> TestEnv {
    let state = Arc::new(InMemoryStateStore::new());
    let (emitter, deliverer) = recording_emitter();
    let enqueuer = Arc::new(FakeEnqueuer::default());
    let deps = Arc::new(Deps {
        cell: Arc::new(RwLock::new(Arc::new(cfg))),
        state: state.clone(),
        locks: RepoLocks::default(),
        emitter,
        enqueuer: enqueuer.clone(),
        test_runner: runner,
    });
    TestEnv {
        deps,
        state,
        deliverer,
        enqueuer,
    }
}
