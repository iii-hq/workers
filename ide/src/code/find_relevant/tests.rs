//! Engine-free asks over a tempdir with a recording fake judge.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use judge_contract::Evaluation;
use serde_json::Value;

use super::navigate::plan_batches;
use super::prompts::{key, FilePreview, Kind, NavigationItem};
use super::*;
use crate::code::judge::Scores;

type Log = Arc<Mutex<Vec<String>>>;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
}

fn fixture(files: &[(&str, &[u8])], configure: impl FnOnce(&Path, &mut CoderConfig)) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let mut cfg = CoderConfig {
        base_paths: vec![root.clone()],
        ..CoderConfig::default()
    };
    configure(&root, &mut cfg);
    Fixture {
        resolver: Arc::new(PathResolver::new(&cfg).unwrap()),
        cfg: Arc::new(cfg),
        root,
        _dir: dir,
    }
}

fn input(query: &str, timeout_ms: u64) -> FindRelevantInput {
    FindRelevantInput {
        query: query.into(),
        path: ".".into(),
        exclude_globs: Vec::new(),
        timeout_ms,
        fs_scope: None,
    }
}

/// A judge that logs every request and answers through `answer`.
fn judge(
    log: &Log,
    answer: impl Fn(&Evaluation) -> Result<Scores, JudgeError> + Send + Sync + 'static,
) -> Evaluator {
    let log = log.clone();
    Arc::new(move |evaluation, _deadline| {
        log.lock()
            .unwrap()
            .push(serde_json::to_string(&evaluation).unwrap());
        let outcome = answer(&evaluation).map(|scores| (scores, 7));
        Box::pin(async move { outcome })
    })
}

/// Score each navigation item by `f`.
fn per_item(ev: &Evaluation, f: impl Fn(&Value) -> f64) -> Scores {
    ev.state["items"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, item)| (key("q", i), f(item)))
        .collect()
}

/// 0.9 for an item whose JSON mentions `needle`, else 0.1.
fn keyword(ev: &Evaluation) -> Result<Scores, JudgeError> {
    Ok(per_item(ev, |item| {
        if item.to_string().contains("needle") {
            0.9
        } else {
            0.1
        }
    }))
}

async fn ask(fx: &Fixture, window: Option<u64>, evaluate: Evaluator) -> FindRelevantOutput {
    ask_with(fx, input("where is the needle?", 120_000), window, evaluate).await
}

async fn ask_with(
    fx: &Fixture,
    req: FindRelevantInput,
    window: Option<u64>,
    evaluate: Evaluator,
) -> FindRelevantOutput {
    run(
        fx.resolver.clone(),
        fx.cfg.clone(),
        req,
        async move { Ok(window) },
        evaluate,
    )
    .await
    .unwrap()
}

fn paths(fx: &Fixture, out: &FindRelevantOutput) -> Vec<String> {
    let root = format!("{}/", fx.root.display());
    out.files
        .iter()
        .map(|f| f.path.strip_prefix(&root).unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn an_irrelevant_branch_is_never_sent() {
    let fx = fixture(
        &[
            ("src/core/needle.rs", b"fn needle() {}\n"),
            ("irrelevant/a/b/c/prune_marker.txt", b"x"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let out = ask(&fx, None, judge(&log, keyword)).await;
    assert_eq!(out.status, Status::Complete);
    assert_eq!(paths(&fx, &out), ["src/core/needle.rs"]);
    assert_eq!(out.stats.judge_calls, 2);
    assert_eq!(out.stats.input_tokens, 14);
    let sent = log.lock().unwrap().join("\n");
    // irrelevant/a was scored (its preview names `b`) and pruned
    assert!(sent.contains("\"irrelevant/a\""));
    assert!(!sent.contains("irrelevant/a/b"));
    assert!(!sent.contains("prune_marker"));
}

#[tokio::test]
async fn nothing_protected_ignored_or_secret_reaches_the_judge() {
    let fx = fixture(
        &[
            ("src/core/needle.rs", b"fn needle() {}\n"),
            (".env", b"SECRET_ENV needle"),
            (".git-credentials", b"SECRET_GITCRED needle"),
            ("id_rsa", b"SECRET_RSA needle"),
            ("x.pem", b"SECRET_PEM needle"),
            ("src/core/credentials.json", b"SECRET_CRED needle"),
            (
                "src/core/key.txt",
                b"-----BEGIN OPENSSH PRIVATE KEY-----\nSECRET_PK needle\n",
            ),
            ("src/core/blob.bin", b"\x00\x01SECRET_BIN needle"),
            ("src/core/latin.txt", b"SECRET_UTF8 needle \xff"),
            (".gitignore", b"ignored.txt\n"),
            ("ignored.txt", b"SECRET_IGN needle"),
            ("node_modules/needle/index.js", b"SECRET_NM needle"),
            ("vendor/needle.go", b"SECRET_VENDOR needle"),
            ("src/core/a.locked", b"SECRET_NA needle"),
            ("denied/needle.txt", b"SECRET_DENY needle"),
        ],
        |root, cfg| {
            cfg.non_accessible_globs = vec!["**/*.locked".into()];
            cfg.denylist_paths = vec![root.join("denied")];
        },
    );
    let log = Log::default();
    let out = ask(&fx, None, judge(&log, keyword)).await;
    assert_eq!(paths(&fx, &out), ["src/core/needle.rs"]);
    let sent = log.lock().unwrap().join("\n");
    assert!(sent.contains("src/core/needle.rs"));
    for forbidden in [
        fx.root.display().to_string().as_str(),
        "SECRET_",
        ".env",
        ".git-credentials",
        ".gitignore",
        "id_rsa",
        "x.pem",
        "credentials.json",
        "ignored.txt",
        "node_modules",
        "vendor",
        "a.locked",
        "denied",
    ] {
        assert!(!sent.contains(forbidden), "{forbidden} reached the judge");
    }
}

fn file_item(path: &str, text: String) -> NavigationItem {
    NavigationItem {
        path: path.into(),
        kind: Kind::File,
        source_range: None,
        file_preview: Some(FilePreview {
            size_bytes: text.len(),
            extension: ".rs".into(),
            preview_bytes: text.len(),
            text,
            truncated: false,
            range: "opening bytes".into(),
            declarations: Some(Vec::new()),
            declaration_index_truncated: Some(false),
        }),
        child_preview: None,
    }
}

#[test]
fn batches_hold_at_most_128_items_and_38000_bytes() {
    let tiny: Vec<_> = (0..300)
        .map(|i| file_item(&format!("f{i}.rs"), "x".into()))
        .collect();
    // the verbatim file question alone is ~560 bytes, so at 38 000 bytes the
    // byte cap binds first; the item cap shows under a larger one
    let (batches, oversize) = plan_batches("q", tiny.clone(), None, usize::MAX);
    assert_eq!(oversize, 0);
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        [128, 128, 44]
    );
    let (batches, _) = plan_batches("q", tiny, None, navigate::MAX_REQUEST_BYTES);
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 300);
    for batch in &batches {
        let bytes = prompts::request_bytes(&prompts::navigation("q", batch, None));
        assert!(bytes <= navigate::MAX_REQUEST_BYTES, "{bytes}");
    }

    let mut big: Vec<_> = (0..7)
        .map(|i| file_item(&format!("f{i}.rs"), "a".repeat(10_000)))
        .collect();
    big.insert(3, file_item("huge.rs", "a".repeat(40_000)));
    let (batches, oversize) = plan_batches("q", big, None, navigate::MAX_REQUEST_BYTES);
    assert_eq!(oversize, 1, "a single item over the cap is dropped");
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [3, 3, 1]);
    for batch in &batches {
        let bytes = prompts::request_bytes(&prompts::navigation("q", batch, None));
        assert!(bytes <= navigate::MAX_REQUEST_BYTES, "{bytes}");
    }
    // a known window caps a request at twice its tokens
    let (batches, _) = plan_batches(
        "q",
        (0..4)
            .map(|i| file_item(&format!("f{i}.rs"), "a".repeat(10_000)))
            .collect(),
        None,
        2 * 10_000,
    );
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1, 1, 1]
    );
}

#[tokio::test]
async fn a_batch_the_judge_finds_too_large_is_split_until_it_fits() {
    let files: Vec<(String, &[u8])> = (0..6)
        .map(|i| (format!("f{i}.rs"), b"needle" as &[u8]))
        .collect();
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), *b)).collect();
    let fx = fixture(&files, |_, _| {});
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        judge(&log, |ev| {
            if ev.questions.len() > 2 {
                Err(JudgeError::TooLarge)
            } else {
                keyword(ev)
            }
        }),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!(out.files.len(), 6);
    // 6 → 3 + 3 → (2 + 1) + (2 + 1)
    assert_eq!(out.stats.judge_calls, 7);

    // a single item the judge refuses is a request-size issue
    let log = Log::default();
    let out = ask(&fx, None, judge(&log, |_| Err(JudgeError::TooLarge))).await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.issues.get("request-size"), Some(&6));
    assert!(out.files.is_empty());
}

#[tokio::test]
async fn admission_is_strictly_above_one_half() {
    let fx = fixture(
        &[
            ("a.rs", b"a"),
            ("b.rs", b"b"),
            ("x/y/z.rs", b"z"),
            ("x/w/v.rs", b"v"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        judge(&log, |ev| {
            Ok(per_item(ev, |item| match item["path"].as_str().unwrap() {
                "a.rs" | "x/y" => 0.5,
                "b.rs" | "x/w" => 0.51,
                _ => 0.9,
            }))
        }),
    )
    .await;
    assert_eq!(paths(&fx, &out), ["x/w/v.rs", "b.rs"]);
    assert!(!log.lock().unwrap().join("\n").contains("x/y/z.rs"));
}

#[tokio::test]
async fn a_judge_down_on_the_first_call_is_unavailable() {
    let fx = fixture(&[("needle.rs", b"needle")], |_, _| {});
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        judge(&log, |_| {
            Err(JudgeError::Unavailable("not registered".into()))
        }),
    )
    .await;
    assert_eq!(out.status, Status::Unavailable);
    assert_eq!(out.reason.as_deref(), Some("not registered"));
    assert_eq!(out.stats.judge_calls, 1);
    assert!(out.files.is_empty());
}

#[tokio::test]
async fn the_deadline_returns_partial_results_as_incomplete() {
    let fx = fixture(
        &[
            ("needle.rs", b"needle"),
            ("deep/more/needle_later.rs", b"needle"),
        ],
        |_, _| {},
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let evaluate: Evaluator = Arc::new(move |ev, _deadline| {
        let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move {
            if !first {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            keyword(&ev).map(|scores| (scores, 1))
        })
    });
    let started = Instant::now();
    let out = ask_with(&fx, input("needle", 1_000), None, evaluate).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("deadline"));
    assert!(out.issues.contains_key("deadline"));
    assert_eq!(paths(&fx, &out), ["needle.rs"]);
}

#[tokio::test]
async fn a_small_window_is_unavailable_without_a_call() {
    let fx = fixture(&[("needle.rs", b"needle")], |_, _| {});
    let log = Log::default();
    let out = ask(&fx, Some(512), judge(&log, keyword)).await;
    assert_eq!(out.status, Status::Unavailable);
    assert_eq!(out.reason.as_deref(), Some("judge window too small"));
    assert_eq!(out.stats.judge_calls, 0);
    assert!(log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bad_input_is_c210() {
    let fx = fixture(&[("f.rs", b"x")], |_, _| {});
    let log = Log::default();
    for (req, message) in [
        (input(" ", 120_000), "query must not be empty"),
        (input(&"q".repeat(4001), 120_000), "at most 4000"),
        (input("q", 999), "timeout_ms"),
        (
            FindRelevantInput {
                path: "f.rs".into(),
                ..input("q", 120_000)
            },
            "not a directory: f.rs",
        ),
    ] {
        let error = run(
            fx.resolver.clone(),
            fx.cfg.clone(),
            req,
            async { Ok(None) },
            judge(&log, keyword),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, CoderError::BadInput(m) if m.contains(message)),
            "{error:?}"
        );
    }
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn files_sort_by_priority_then_score_then_path() {
    let file = |path: &str, score: f64, priority: Option<f64>| RelevantFile {
        path: path.into(),
        score,
        priority,
        roles: Vec::new(),
        excerpts: Vec::new(),
        leads: Vec::new(),
        call_leads: Vec::new(),
        source_omitted: false,
    };
    let mut files = vec![
        file("/c", 0.9, None),
        file("/b", 0.6, Some(0.95)),
        file("/a", 0.9, None),
        file("/d", 0.99, Some(0.2)),
    ];
    sort_files(&mut files);
    let order: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(order, ["/b", "/a", "/c", "/d"]);
}
