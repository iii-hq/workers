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
        let evaluation = prompts::decoded(evaluation);
        log.lock()
            .unwrap()
            .push(serde_json::to_string(&evaluation).unwrap());
        let outcome = answer(&evaluation).map(|scores| (scores, 7));
        Box::pin(async move { outcome })
    })
}

/// Score each navigation item by `f`; select no evidence.
fn per_item(ev: &Evaluation, f: impl Fn(&Value) -> f64) -> Scores {
    let Some(items) = ev.state["items"].as_array() else {
        return ev.questions.keys().map(|k| (k.clone(), 0.1)).collect();
    };
    items
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
        move |_| async move { Ok(window) },
        evaluate,
        None,
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
    // two navigation levels, then one evidence call and one assessment
    // for the file
    assert_eq!(out.stats.judge_calls, 4);
    assert_eq!(out.stats.input_tokens, 28);
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
            (
                "src/core/key.asc",
                b"-----BEGIN PGP PRIVATE KEY BLOCK-----\nSECRET_PGP needle\n",
            ),
            ("src/core/blob.bin", b"\x00\x01SECRET_BIN needle"),
            ("src/core/latin.txt", b"SECRET_UTF8 needle \xff"),
            // a whitelisted dot-folder is still hidden
            (".gitignore", b"ignored.txt\n!.secretdir/\n"),
            (".secretdir/needle.rs", b"SECRET_HIDDEN needle"),
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
        ".secretdir",
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
            if ev.state.get("items").is_some() && ev.questions.len() > 2 {
                Err(JudgeError::TooLarge)
            } else {
                keyword(ev)
            }
        }),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!(out.files.len(), 6);
    // 6 → 3 + 3 → (2 + 1) + (2 + 1), then one evidence call and one
    // assessment per file
    assert_eq!(out.stats.judge_calls, 7 + 6 + 6);

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
            keyword(&prompts::decoded(ev)).map(|scores| (scores, 1))
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
            |_| async { Ok(None) },
            judge(&log, keyword),
            None,
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

#[tokio::test]
async fn excerpts_past_the_output_budget_are_omitted_but_keep_their_leads() {
    // three ~50 000-byte files of ten functions the judge selects whole:
    // two fit in 131 072 source bytes (and in the result cap)
    let filler = "    // filler filler filler filler filler filler\n".repeat(100);
    let source: String = (0..10)
        .map(|i| format!("fn needle_{i:04}() -> u32 {{\n{filler}    {i}\n}}\n"))
        .collect();
    let fx = fixture(
        &[
            ("a_needle.rs", source.as_bytes()),
            ("b_needle.rs", source.as_bytes()),
            ("c_needle.rs", source.as_bytes()),
        ],
        |_, _| {},
    );
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            if ev.state.get("items").is_some() {
                keyword(ev)
            } else {
                Ok(ev.questions.keys().map(|k| (k.clone(), 0.9)).collect())
            }
        }),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!(
        paths(&fx, &out),
        ["a_needle.rs", "b_needle.rs", "c_needle.rs"]
    );
    let shown: Vec<usize> = out
        .files
        .iter()
        .map(|f| f.excerpts.iter().map(|e| e.text.len()).sum())
        .collect();
    assert_eq!(shown[0], source.len());
    assert_eq!(shown, [shown[0], shown[0], 0]);
    let omitted: Vec<bool> = out.files.iter().map(|f| f.source_omitted).collect();
    assert_eq!(omitted, [false, false, true]);
    assert_eq!(out.files[2].leads.len(), out.files[0].leads.len());
    assert_eq!(out.files[2].leads[0].name, "needle_0000");
}

/// What harness/src/trigger.rs `cap_result` measures against its 262_144
/// default: the result as one JSON text block, beside the result itself.
fn harness_bytes(out: &FindRelevantOutput) -> usize {
    let text = serde_json::to_string(out).unwrap();
    let content = serde_json::json!([{ "type": "text", "text": text }]);
    serde_json::to_vec(&(content, out)).unwrap().len()
}

#[test]
fn a_result_stays_under_the_harness_cap_whatever_it_escapes() {
    // quotes and backslashes count 6 times once escaped twice, tabs and
    // newlines 5
    let nasty = "\"\\\t\n\r".repeat(480);
    let file = |i: usize| RelevantFile {
        path: format!("/{}/{i:04}\"\\.py", "d".repeat(200)),
        score: 0.9,
        priority: Some(0.5),
        roles: vec!["implementation".into()],
        excerpts: (0..30u32)
            .map(|j| Excerpt {
                line_from: j,
                line_to: j,
                text: nasty.clone(),
                partial: None,
            })
            .collect(),
        leads: (0..50u32)
            .map(|j| Lead {
                name: format!("lead\"{j}"),
                line_from: j,
                line_to: j,
                score: 0.5,
            })
            .collect(),
        call_leads: (0..20u32)
            .map(|j| CallLead {
                caller: "A.go".into(),
                name: format!("B.m{j}"),
                line_from: j,
                line_to: j,
                unknown_earlier_bases: vec!["External".into()],
            })
            .collect(),
        source_omitted: false,
    };
    let output = |files: usize| FindRelevantOutput {
        status: Status::Complete,
        reason: None,
        files: (0..files).map(file).collect(),
        agents_md: vec!["/r/AGENTS.md".into()],
        issues: BTreeMap::new(),
        stats: Stats::default(),
    };

    // a few files: their escaped source fills the cap long before 131 072
    // source bytes
    let mut few = output(3);
    assert!(harness_bytes(&few) > 262_144);
    spend_budget(&mut few, MAX_SOURCE_BYTES, MAX_RESULT_BYTES);
    assert!(harness_bytes(&few) < 262_144);
    let shown: usize = few
        .files
        .iter()
        .flat_map(|f| &f.excerpts)
        .map(|e| e.text.len())
        .sum();
    assert!(shown > 30_000 && shown < MAX_SOURCE_BYTES / 2, "{shown}");
    assert!(few.files[2].source_omitted && few.files[2].excerpts.is_empty());
    assert_eq!(few.files[2].leads.len(), 50);
    assert_eq!(few.status, Status::Complete);

    // many files: even their paths overflow, so the last ones go, and the
    // top files keep their leads and some source
    let mut many = output(400);
    spend_budget(&mut many, MAX_SOURCE_BYTES, MAX_RESULT_BYTES);
    assert!(harness_bytes(&many) < 262_144);
    assert!(many.files.len() < 400);
    assert_eq!(many.files[0].leads.len(), 50);
    assert!(!many.files[0].excerpts.is_empty());
    assert_eq!(many.status, Status::Incomplete);
    assert_eq!(many.issues.get("resource_limit"), Some(&1));

    // live shape (122 files, ~1300 leads): every path stays, the tail loses
    // its leads, and the top files still show source
    let plain = |i: usize| RelevantFile {
        path: format!("/r/src/module_{i:03}.rs"),
        excerpts: (0..4u32)
            .map(|j| Excerpt {
                line_from: j * 40 + 1,
                line_to: j * 40 + 30,
                text: "    let value = compute(input);\n".repeat(30),
                partial: None,
            })
            .collect(),
        leads: (0..11u32)
            .map(|j| Lead {
                name: format!("Type{i}.method_{j}"),
                line_from: j * 10 + 1,
                line_to: j * 10 + 9,
                score: 0.4,
            })
            .collect(),
        call_leads: Vec::new(),
        ..file(i)
    };
    let mut leady = FindRelevantOutput {
        files: (0..122).map(plain).collect(),
        ..output(0)
    };
    spend_budget(&mut leady, MAX_SOURCE_BYTES, MAX_RESULT_BYTES);
    assert!(harness_bytes(&leady) < 262_144);
    assert_eq!(leady.files.len(), 122);
    assert_eq!(leady.files[0].leads.len(), 11);
    assert!(!leady.files[0].excerpts.is_empty());
    assert!(leady.files[121].leads.is_empty());
    assert_eq!(leady.issues.get("resource_limit"), Some(&1));
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

#[tokio::test]
async fn git_metadata_is_never_a_walk_root() {
    let fx = fixture(
        &[
            (".git/config", b"url = https://user:TOKEN@host/repo\n"),
            (".git/logs/HEAD", b"TOKEN"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    for path in [".git", ".git/logs"] {
        let error = run(
            fx.resolver.clone(),
            fx.cfg.clone(),
            FindRelevantInput {
                path: path.into(),
                ..input("q", 120_000)
            },
            |_| async { Ok(None) },
            judge(&log, keyword),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, CoderError::BadInput(m) if m.contains(".git")),
            "{error:?}"
        );
    }
    assert!(log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_preview_too_big_for_the_window_is_scored_in_chunks_keeping_the_best() {
    // 375 lines of 80 bytes: three 12 000-byte chunks, `alpha` in the first,
    // `needle` in the last
    let line = |word: &str| format!("{word:<79}\n");
    let mut source = line("alpha");
    for i in 2..=375 {
        source.push_str(&line(if i == 301 { "needle" } else { "filler" }));
    }
    let fx = fixture(&[("big.txt", source.as_bytes())], |_, _| {});
    let log = Log::default();
    let out = ask(
        &fx,
        Some(8_192),
        judge(&log, |ev| {
            Ok(per_item(ev, |item| {
                let text = item.to_string();
                if text.contains("needle") {
                    0.9
                } else if text.contains("alpha") {
                    0.7
                } else {
                    0.1
                }
            }))
        }),
    )
    .await;
    // the whole preview is over 2 × 8192 bytes, too big to assess
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(
        out.issues,
        BTreeMap::from([("request-size".to_string(), 1)])
    );
    assert_eq!(paths(&fx, &out), ["big.txt"]);
    assert_eq!(out.files[0].score, 0.9);
    let log = log.lock().unwrap();
    let navigation: Vec<_> = log
        .iter()
        .filter(|sent| sent.contains("\"items\""))
        .collect();
    assert_eq!(navigation.len(), 3);
    assert!(navigation
        .iter()
        .all(|sent| sent.contains("sampled source ranges")));
}

#[tokio::test]
async fn exclude_globs_prune_folders_and_files() {
    let fx = fixture(
        &[
            ("needle.rs", b"needle"),
            ("gen/needle.rs", b"needle"),
            ("made/deep/needle.rs", b"needle"),
            ("notes/needle.txt", b"needle"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let req = FindRelevantInput {
        exclude_globs: vec!["gen/".into(), "made/**".into(), "**/*.txt".into()],
        ..input("where is the needle?", 120_000)
    };
    let out = ask_with(&fx, req, None, judge(&log, keyword)).await;
    assert_eq!(paths(&fx, &out), ["needle.rs"]);
    let sent = log.lock().unwrap().join("\n");
    for forbidden in ["\"gen\"", "\\\"gen", "\"made", "\\\"made", "needle.txt"] {
        assert!(!sent.contains(forbidden), "{forbidden} reached the judge");
    }
}

#[tokio::test]
async fn a_judge_not_ready_is_unavailable_without_a_call() {
    let fx = fixture(&[("needle.rs", b"needle")], |_, _| {});
    let log = Log::default();
    let out = run(
        fx.resolver.clone(),
        fx.cfg.clone(),
        input("needle", 120_000),
        |_| async { Err(JudgeError::Unavailable("judge provider not ready".into())) },
        judge(&log, keyword),
        None,
    )
    .await
    .unwrap();
    assert_eq!(out.status, Status::Unavailable);
    assert_eq!(out.reason.as_deref(), Some("judge provider not ready"));
    assert!(log.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn an_unlistable_folder_makes_the_result_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture(
        &[("needle.rs", b"needle"), ("locked/x.rs", b"x")],
        |_, _| {},
    );
    let locked = fx.root.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let listable = std::fs::read_dir(&locked).is_ok(); // root ignores modes
    let out = ask(&fx, None, judge(&Log::default(), keyword)).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if listable {
        return;
    }
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.issues.get("unreadable"), Some(&1));
    assert_eq!(paths(&fx, &out), ["needle.rs"]);
}

#[tokio::test]
async fn the_entry_cap_counts_only_what_discovery_lists() {
    // `aaa/heavy` alone holds more than MAX_ENTRIES names: a depth-first
    // walk capped there never reached `zzz`, though the judge prunes it
    let fx = fixture(&[("zzz/needle.rs", b"fn needle() {}\n")], |_, _| {});
    let heavy = fx.root.join("aaa/heavy");
    std::fs::create_dir_all(&heavy).unwrap();
    for i in 0..=walk::MAX_ENTRIES {
        std::fs::File::create(heavy.join(format!("f{i:06}"))).unwrap();
    }
    let log = Log::default();
    let out = ask(&fx, None, judge(&log, keyword)).await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!(paths(&fx, &out), ["zzz/needle.rs"]);
    // `aaa/heavy` was offered on its (truncated) preview and pruned
    let heavy_item = sent(&log)
        .into_iter()
        .flat_map(|ev| ev["state"]["items"].as_array().cloned().unwrap_or_default())
        .find(|item| item["path"] == "aaa/heavy")
        .unwrap();
    assert_eq!(heavy_item["childPreview"]["truncated"], true);
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_after_the_walk_is_never_read() {
    let fx = fixture(&[("src/a.rs", b"inside")], |_, _| {});
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("a.rs"), "SECRET_OUTSIDE").unwrap();
    let tree = walk::Tree::new(&fx.resolver, &fx.root, None, u64::MAX);
    assert!(matches!(walk::read(&tree, "src/a.rs"), walk::Snap::Ok(_)));
    std::fs::rename(fx.root.join("src"), fx.root.join("old")).unwrap();
    std::os::unix::fs::symlink(outside.path(), fx.root.join("src")).unwrap();
    assert!(matches!(
        walk::read(&tree, "src/a.rs"),
        walk::Snap::Issue("changed")
    ));
}

#[test]
fn names_sort_like_locale_compare() {
    let mut names = vec!["B", "a", "_x", "A", "b", "1"];
    names.sort_by(|a, b| walk::locale_cmp(a, b));
    assert_eq!(names, ["1", "_x", "a", "A", "b", "B"]);
}

fn sent(log: &Log) -> Vec<Value> {
    log.lock()
        .unwrap()
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

/// Navigation by [`keyword`]; evidence by `answer(name, follow_up)` →
/// `(q, ref)` with scope 1; anything else 0.1.
fn by_declaration(
    ev: &Evaluation,
    answer: impl Fn(&str, bool) -> (f64, f64),
) -> Result<Scores, JudgeError> {
    let Some(declarations) = ev.state["declarations"].as_array() else {
        return keyword(ev);
    };
    let follow_up = ev.state.get("selectedEvidence").is_some();
    let mut scores = Scores::new();
    for (i, d) in declarations.iter().enumerate() {
        let (q, reference) = answer(d["name"].as_str().unwrap(), follow_up);
        scores.insert(key("q", i), q);
        scores.insert(key("scope", i), 1.0);
        if follow_up {
            scores.insert(key("ref", i), reference);
        }
    }
    Ok(scores)
}

const TWO_FUNCTIONS: &[u8] =
    b"fn needle() -> u32 {\n    helper()\n}\n\n\n\n\n\n\n\nfn helper() -> u32 {\n    7\n}\n";

fn texts(file: &RelevantFile) -> String {
    file.excerpts.iter().map(|e| e.text.as_str()).collect()
}

#[tokio::test]
async fn the_follow_up_asks_references_and_retracts_rejected_selections() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        judge(&log, |ev| {
            by_declaration(ev, |name, follow_up| match (name, follow_up) {
                ("needle", false) => (0.9, 0.0),
                // valid rejection: min(q, scope) and ref both ≤ 0.5
                ("needle", true) => (0.2, 0.1),
                // selected on the reference alone: max(min(0.1, 1), 0.9)
                ("helper", true) => (0.1, 0.9),
                _ => (0.1, 0.0),
            })
        }),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    let file = &out.files[0];
    assert!(texts(file).contains("fn helper"));
    assert!(!texts(file).contains("fn needle"));
    // leads keep their best standing: the rejection scored ≤ 0.25
    let leads: Vec<_> = file
        .leads
        .iter()
        .map(|l| (l.name.as_str(), l.score))
        .collect();
    assert_eq!(leads, [("needle", 0.9), ("helper", 0.9)]);

    let follow_ups: Vec<Value> = sent(&log)
        .into_iter()
        .filter(|ev| ev["state"].get("selectedEvidence").is_some())
        .collect();
    assert_eq!(follow_ups.len(), 1);
    let shared = &follow_ups[0]["state"]["selectedEvidence"];
    assert_eq!(shared[0]["path"], "needle.rs");
    assert_eq!(shared[0]["startLine"], 1);
    assert!(shared[0]["source"]
        .as_str()
        .unwrap()
        .starts_with("fn needle"));
    assert!(follow_ups[0]["questions"].get("ref001").is_some());
}

#[tokio::test]
async fn a_failed_follow_up_keeps_the_first_pass_evidence() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            if ev.state.get("selectedEvidence").is_some() {
                return Err(JudgeError::Rejected("invalid_request".into()));
            }
            by_declaration(ev, |name, _| {
                (if name == "needle" { 0.9 } else { 0.1 }, 0.0)
            })
        }),
    )
    .await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.issues.get("invalid_request"), Some(&1));
    assert!(texts(&out.files[0]).contains("fn needle"));
    assert!(!texts(&out.files[0]).contains("fn helper"));
}

#[tokio::test]
async fn roles_and_priority_order_files_and_a_test_file_shows_all_its_selection() {
    let source = b"fn needle_a() {}\n\n\n\n\n\n\n\n\nfn needle_b() {}\n";
    let fx = fixture(
        &[
            ("impl_needle.rs", source),
            ("b_needle.rs", source),
            ("t_needle.rs", source),
        ],
        |_, _| {},
    );
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            if !ev.questions.contains_key("priority") {
                // needle_b is selected (0.6) but not presented (≤ 0.7)
                return by_declaration(ev, |name, _| {
                    (if name == "needle_a" { 0.9 } else { 0.6 }, 0.0)
                });
            }
            let answers: &[(&str, f64)] = match ev.state["path"].as_str().unwrap() {
                "impl_needle.rs" => &[("implementation", 0.9), ("helper", 0.6), ("priority", 0.3)],
                "t_needle.rs" => &[("test", 0.9), ("caller", 0.5), ("priority", 0.95)],
                _ => return Err(JudgeError::Rejected("invalid_request".into())),
            };
            Ok(ev
                .questions
                .keys()
                .map(|id| {
                    let p = answers
                        .iter()
                        .find(|(role, _)| role == id)
                        .map_or(0.1, |a| a.1);
                    (id.clone(), p)
                })
                .collect())
        }),
    )
    .await;
    // priority 0.95, then b's score 0.9 (unassessed), then priority 0.3
    assert_eq!(
        paths(&fx, &out),
        ["t_needle.rs", "b_needle.rs", "impl_needle.rs"]
    );
    let summary: Vec<_> = out
        .files
        .iter()
        .map(|f| (f.roles.clone(), f.priority))
        .collect();
    assert_eq!(
        summary,
        [
            (vec!["test".to_string()], Some(0.95)),
            (vec![], None),
            (
                vec!["implementation".to_string(), "helper".to_string()],
                Some(0.3)
            ),
        ]
    );
    assert!(texts(&out.files[0]).contains("fn needle_b"));
    assert!(!texts(&out.files[2]).contains("fn needle_b"));
    assert!(texts(&out.files[2]).contains("fn needle_a"));
}

#[tokio::test]
async fn the_relationship_pass_rediscovers_a_pruned_directory_once() {
    let fx = fixture(
        &[
            (
                "a/impl/needle.ts",
                b"export class Needle {\n  run() {\n    return 1;\n  }\n}\n",
            ),
            (
                "b/ext/plugin.ts",
                b"import { Needle } from \"../../a/impl/needle\";\nexport class Plugin extends Needle {}\n",
            ),
            ("c/other/thing.ts", b"export const thing = 1;\n"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        judge(&log, |ev| {
            if ev.state.get("relationAnchor").is_none() {
                return keyword(ev);
            }
            Ok(per_item(ev, |item| {
                if item.to_string().contains("extends Needle") {
                    0.9
                } else {
                    0.1
                }
            }))
        }),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!(paths(&fx, &out), ["a/impl/needle.ts", "b/ext/plugin.ts"]);
    let anchored: Vec<Value> = sent(&log)
        .into_iter()
        .filter(|ev| ev["state"].get("relationAnchor").is_some())
        .collect();
    // one re-score of the pruned directories, one anchored discovery
    assert_eq!(anchored.len(), 2);
    let rescore = &anchored[0]["state"];
    assert_eq!(
        rescore["relationAnchor"],
        serde_json::json!({"path": "a/impl/needle.ts", "classes": ["Needle"]})
    );
    let dirs: Vec<&str> = rescore["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert_eq!(dirs, ["b/ext", "c/other"]);
    let sample = &rescore["items"][0]["childPreview"]["contentSamples"][0];
    assert_eq!(sample["name"], "plugin.ts");
    assert_eq!(sample["truncated"], false);
    assert_eq!(anchored[1]["state"]["items"][0]["path"], "b/ext/plugin.ts");
}

#[test]
fn content_samples_take_head_middle_and_tail_and_shrink_to_fit() {
    let files: Vec<(String, Vec<u8>)> = (0..2)
        .map(|i| (format!("d/s/f{i}.txt"), "x".repeat(30_000).into_bytes()))
        .collect();
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), &b[..])).collect();
    let fx = fixture(&files, |_, _| {});
    let run = Run {
        query: "q".into(),
        tree: walk::Tree::new(&fx.resolver, &fx.root, None, u64::MAX),
        evaluate: judge(&Log::default(), keyword),
        deadline: Instant::now() + Duration::from_secs(60),
        cap: usize::MAX,
        state_cap: usize::MAX,
        window_cap: usize::MAX,
        cache: None,
        slots: crate::code::judge::DEFAULT_SLOTS,
        token_budget: 0,
        state: Mutex::new(Default::default()),
    };
    let item = NavigationItem {
        path: "d/s".into(),
        kind: Kind::Directory,
        source_range: None,
        file_preview: None,
        child_preview: walk::preview_directory(&run.tree, "d/s"),
    };
    let preview = run.with_directory_content(item).child_preview.unwrap();
    let samples = preview.content_samples.as_ref().unwrap();
    // 8000 per file, three 2666-unit parts at 0, 15000 - 1333 and the tail
    let offsets: Vec<&str> = samples[0]
        .source
        .lines()
        .filter(|l| l.starts_with('['))
        .collect();
    assert_eq!(
        offsets,
        [
            "[character offset 0]",
            "[character offset 13667]",
            "[character offset 27334]"
        ]
    );
    assert!(samples.iter().all(|s| s.truncated));
    assert!(walk::json_len(&preview) <= 28_000);

    // many files: the 80-unit floor, then shrinking by 4/5 until it fits
    let files: Vec<(String, Vec<u8>)> = (0..60)
        .map(|i| (format!("m/s/f{i:02}.txt"), "y".repeat(2_000).into_bytes()))
        .collect();
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), &b[..])).collect();
    let fx = fixture(&files, |_, _| {});
    let run = Run {
        tree: walk::Tree::new(&fx.resolver, &fx.root, None, u64::MAX),
        ..run
    };
    let item = NavigationItem {
        path: "m/s".into(),
        child_preview: walk::preview_directory(&run.tree, "m/s"),
        ..NavigationItem {
            path: String::new(),
            kind: Kind::Directory,
            source_range: None,
            file_preview: None,
            child_preview: None,
        }
    };
    let preview = run.with_directory_content(item).child_preview.unwrap();
    let samples = preview.content_samples.as_ref().unwrap();
    assert_eq!(samples.len(), 60);
    assert!(walk::json_len(&preview) <= 28_000);
    assert!(samples.iter().all(|s| s.source.len() >= 80));
}

#[tokio::test]
async fn answers_are_reused_across_asks_in_one_namespace() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    // `fail` rejects the file assessment, which must not be cached
    let judge_for = |log: &Log, fail: bool| {
        judge(log, move |ev| {
            if fail && ev.questions.contains_key("priority") {
                return Err(JudgeError::Rejected("invalid_request".into()));
            }
            by_declaration(ev, |name, _| {
                (if name == "needle" { 0.9 } else { 0.1 }, 0.0)
            })
        })
    };
    let namespace = format!("test-{}", std::process::id());
    let ask_cached = |log: &Log, fail: bool| {
        run(
            fx.resolver.clone(),
            fx.cfg.clone(),
            input("where is the needle?", 120_000),
            |_| async { Ok(None) },
            judge_for(log, fail),
            Some(namespace.clone()),
        )
    };
    let logs = [Log::default(), Log::default(), Log::default()];
    let first = ask_cached(&logs[0], true).await.unwrap();
    let second = ask_cached(&logs[1], false).await.unwrap();
    let third = ask_cached(&logs[2], false).await.unwrap();
    assert!(first.stats.judge_calls > 0);
    assert_eq!(first.stats.cache_hits, 0);
    assert_eq!(first.issues.get("invalid_request"), Some(&1));
    // only the failed request is asked again
    assert_eq!(second.stats.judge_calls, 1);
    assert_eq!(second.stats.cache_hits, first.stats.judge_calls - 1);
    let asked = sent(&logs[1]);
    assert_eq!(asked.len(), 1);
    assert!(asked[0]["questions"].get("priority").is_some());
    assert_eq!(third.stats.judge_calls, 0);
    assert_eq!(third.stats.cache_hits, first.stats.judge_calls);
    assert!(logs[2].lock().unwrap().is_empty());
    assert_eq!(texts(&first.files[0]), texts(&third.files[0]));
    assert_eq!(third.status, Status::Complete);
}

#[test]
fn the_cache_rejects_invalid_answers_and_keeps_within_its_caps() {
    use judge_contract::{Content, Question};
    let questions = |ids: &[&str]| -> BTreeMap<String, Question> {
        ids.iter()
            .map(|id| {
                let question = Question::Noul {
                    instructions: Content::Text("x".into()),
                    criteria: None,
                };
                (id.to_string(), question)
            })
            .collect()
    };
    let scores = |pairs: &[(&str, f64)]| -> Scores {
        pairs.iter().map(|(id, p)| (id.to_string(), *p)).collect()
    };
    // a one-answer entry's resident cost, not its 12 JSON bytes
    let entry = CACHE_ENTRY_OVERHEAD + CACHE_ANSWER_OVERHEAD;
    let mut cache = AnswerCache::new(7 * entry + entry / 2, 40);
    cache.put([1; 32], &scores(&[("q000", 0.5)])); // 12 JSON bytes
    assert_eq!(
        cache.get(&[1; 32], &questions(&["q000"])),
        Some(scores(&[("q000", 0.5)]))
    );
    // answers to other questions are no hit
    assert_eq!(cache.get(&[1; 32], &questions(&["q001"])), None);
    assert_eq!(cache.get(&[1; 32], &questions(&["q000", "q001"])), None);
    // out-of-range answers and entries over the entry cap are never kept
    cache.put([2; 32], &scores(&[("q000", 1.5)]));
    cache.put([3; 32], &scores(&[("q000", f64::NAN)]));
    let many: Vec<(String, f64)> = (0..4).map(|i| (key("q", i), 0.1)).collect();
    let many: Scores = many.into_iter().collect();
    cache.put([4; 32], &many);
    for key in [[2; 32], [3; 32], [4; 32]] {
        assert!(!cache.entries.contains_key(&key));
    }
    // past the total, the oldest entries go first
    for i in 5..=12 {
        cache.put([i; 32], &scores(&[("q000", 0.25)]));
    }
    assert!(cache.bytes <= cache.max_bytes);
    assert!(!cache.entries.contains_key(&[1; 32]));
    assert!(!cache.entries.contains_key(&[5; 32]));
    assert!((6..=12).all(|i| cache.entries.contains_key(&[i; 32])));
    assert_eq!(cache.bytes, 7 * entry);
}

#[tokio::test]
async fn agents_md_lists_accessible_files_at_the_root_and_above_returned_files() {
    let fx = fixture(
        &[
            ("AGENTS.md", b"root rules"),
            ("src/AGENTS.md", b"SECRET_RULES"),
            ("src/core/AGENTS.md", b"core rules"),
            ("src/core/needle.rs", b"fn needle() {}\n"),
            ("other/AGENTS.md", b"other rules"),
        ],
        |_, cfg| cfg.non_accessible_globs = vec!["src/AGENTS.md".into()],
    );
    let log = Log::default();
    let out = ask(&fx, None, judge(&log, keyword)).await;
    assert_eq!(paths(&fx, &out), ["src/core/needle.rs"]);
    let root = fx.root.display();
    assert_eq!(
        out.agents_md,
        [
            format!("{root}/AGENTS.md"),
            format!("{root}/src/core/AGENTS.md")
        ]
    );
    assert!(!log.lock().unwrap().join("\n").contains("SECRET_RULES"));
}

#[tokio::test]
async fn a_follow_up_reply_missing_an_answer_retracts_nothing() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            let mut scores = by_declaration(ev, |name, follow_up| match (name, follow_up) {
                ("needle", false) => (0.9, 0.0),
                // would retract `needle`, but `ref000` goes missing
                _ => (0.1, 0.1),
            })?;
            scores.remove("ref000");
            Ok(scores)
        }),
    )
    .await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("invalid_response"));
    assert!(texts(&out.files[0]).contains("fn needle"));
}

#[tokio::test]
async fn a_small_window_caps_the_shared_evidence() {
    let padding = "    // padding padding padding padding padding padding padding padding\n";
    let source = format!("fn needle() {{\n{}}}\n", padding.repeat(120));
    let fx = fixture(
        &[
            ("a_needle.rs", source.as_bytes()),
            ("b_needle.rs", source.as_bytes()),
        ],
        |_, _| {},
    );
    // ~17 KB of shared evidence: under jevgrep's 64 000, over 2 × 8192
    for (window, follow_ups) in [(None, 2), (Some(8_192), 0)] {
        let log = Log::default();
        let out = ask(
            &fx,
            window,
            judge(&log, |ev| by_declaration(ev, |_, _| (0.9, 0.0))),
        )
        .await;
        assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
        assert_eq!(out.files.len(), 2);
        let shared = sent(&log)
            .iter()
            .filter(|ev| ev["state"].get("selectedEvidence").is_some())
            .count();
        assert_eq!(shared, follow_ups, "window {window:?}");
    }
}

#[tokio::test]
async fn a_file_that_changes_under_its_assessment_keeps_no_roles() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    let path = fx.root.join("needle.rs");
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), move |ev| {
            if !ev.questions.contains_key("priority") {
                return by_declaration(ev, |name, _| {
                    (if name == "needle" { 0.9 } else { 0.1 }, 0.0)
                });
            }
            std::fs::write(&path, b"fn changed() {}\n").unwrap();
            Ok(ev.questions.keys().map(|id| (id.clone(), 0.9)).collect())
        }),
    )
    .await;
    let file = &out.files[0];
    assert!(file.roles.is_empty());
    assert_eq!(file.priority, None);
    assert!(file.source_omitted);
    assert!(file.excerpts.is_empty() && file.leads.is_empty());
    assert!(out.issues.contains_key("changed"));
}

#[tokio::test]
async fn nothing_secret_in_a_pruned_directory_reaches_the_anchored_judge() {
    let fx = fixture(
        &[
            (
                "a/impl/needle.ts",
                b"export class Needle {\n  run() {\n    return 1;\n  }\n}\n",
            ),
            (
                "b/ext/plugin.ts",
                b"export class Plugin extends Needle {}\n",
            ),
            (
                "b/ext/key.txt",
                b"-----BEGIN OPENSSH PRIVATE KEY-----\nSECRET_PK extends Needle\n",
            ),
            ("b/ext/blob.bin", b"\x00\x01SECRET_BIN extends Needle"),
            ("b/ext/latin.txt", b"SECRET_UTF8 extends Needle \xff"),
            ("b/ext/a.locked", b"SECRET_NA extends Needle"),
            ("b/ext/ignored.txt", b"SECRET_IGN extends Needle"),
            ("b/ext/.env", b"SECRET_ENV extends Needle"),
            ("b/ext/id_rsa", b"SECRET_RSA extends Needle"),
            (".gitignore", b"ignored.txt\n"),
        ],
        |_, cfg| cfg.non_accessible_globs = vec!["**/*.locked".into()],
    );
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        judge(&log, |ev| {
            if ev.state.get("relationAnchor").is_none() {
                return keyword(ev);
            }
            Ok(per_item(ev, |item| {
                if item.to_string().contains("extends Needle") {
                    0.9
                } else {
                    0.1
                }
            }))
        }),
    )
    .await;
    assert_eq!(paths(&fx, &out), ["a/impl/needle.ts", "b/ext/plugin.ts"]);
    let anchored: Vec<String> = sent(&log)
        .iter()
        .filter(|ev| ev["state"].get("relationAnchor").is_some())
        .map(Value::to_string)
        .collect();
    assert!(anchored[0].contains("contentSamples"));
    let sent = log.lock().unwrap().join("\n");
    for forbidden in [
        fx.root.display().to_string().as_str(),
        "SECRET_",
        "a.locked",
        "ignored.txt",
        ".env",
        "id_rsa",
    ] {
        assert!(!sent.contains(forbidden), "{forbidden} reached the judge");
    }
}

fn ranges(file: &RelevantFile) -> Vec<(u32, u32)> {
    file.excerpts
        .iter()
        .map(|e| (e.line_from, e.line_to))
        .collect()
}

/// Navigation by [`keyword`], evidence by `select(name)` (the same in the
/// follow-up), the assessment by `roles` (the rest 0.1) and test bodies by
/// `keep(name)`.
fn python_judge(
    log: &Log,
    select: fn(&str) -> f64,
    roles: &'static [(&'static str, f64)],
    keep: fn(&str) -> f64,
) -> Evaluator {
    judge(log, move |ev| {
        if let Some(candidates) = ev.state.get("candidates") {
            return Ok(candidates
                .as_object()
                .unwrap()
                .values()
                .enumerate()
                .map(|(i, c)| (key("q", i), keep(c["name"].as_str().unwrap())))
                .collect());
        }
        if ev.questions.contains_key("priority") {
            return Ok(ev
                .questions
                .keys()
                .map(|id| {
                    let p = roles.iter().find(|(r, _)| r == id).map_or(0.1, |r| r.1);
                    (id.clone(), p)
                })
                .collect());
        }
        by_declaration(ev, |name, _| (select(name), 0.0))
    })
}

const CALLER: &[u8] = b"class Base:\n    def run(self):\n        return 0\n\n\n\n\n\n\n\n\n\n\nclass Needle(Base):\n    def go(self):\n        return self.run()\n";

#[tokio::test]
async fn a_selected_python_method_shows_the_local_method_it_calls() {
    let fx = fixture(&[("needle.py", CALLER)], |_, _| {});
    let out = ask(
        &fx,
        None,
        python_judge(
            &Log::default(),
            |name| if name == "Needle.go" { 0.9 } else { 0.1 },
            &[],
            |_| 0.0,
        ),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    let file = &out.files[0];
    // Base.run (2-3) and Base's header (1) join the presented Needle.go
    assert_eq!(ranges(file), [(1, 3), (11, 17)]);
    assert!(file.excerpts[0].text.ends_with("        return 0"));
    let leads: Vec<_> = file
        .call_leads
        .iter()
        .map(|c| {
            format!(
                "Possible local call {} -> {}: lines {}-{}",
                c.caller, c.name, c.line_from, c.line_to
            )
        })
        .collect();
    assert_eq!(
        leads,
        ["Possible local call Needle.go -> Base.run: lines 2-3"]
    );
    assert!(file.call_leads[0].unknown_earlier_bases.is_empty());
}

const TESTS: &[u8] = b"import pytest\n\n\ndef test_keep():\n    assert 1\n\n\ndef test_drop():\n    assert 2\n\n\ndef test_other():\n    assert 3\n";

const TEST_ROLE: &[(&str, f64)] = &[("test", 0.9), ("priority", 0.5)];

#[tokio::test]
async fn a_python_test_file_drops_the_bodies_the_judge_declines() {
    let fx = fixture(&[("test_needle.py", TESTS)], |_, _| {});
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        python_judge(
            &log,
            |_| 0.9,
            TEST_ROLE,
            |name| {
                if name == "test_keep" {
                    0.9
                } else {
                    0.2
                }
            },
        ),
    )
    .await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    let file = &out.files[0];
    assert_eq!(file.roles, ["test"]);
    // lines 1-14 less test_drop (8-9) and test_other (12-13); the blank
    // remainders go too
    assert_eq!(ranges(file), [(1, 7)]);
    assert!(file.excerpts[0].text.ends_with("    assert 1\n\n"));
    // every body stays a lead
    assert_eq!(file.leads.len(), 3);
    let asked: Vec<Value> = sent(&log)
        .into_iter()
        .filter(|ev| ev["state"].get("candidates").is_some())
        .collect();
    assert_eq!(asked.len(), 1);
    let candidates = &asked[0]["state"]["candidates"];
    assert_eq!(candidates["c001"]["name"], "test_drop");
    assert_eq!(
        candidates["c001"]["source"],
        "def test_drop():\n    assert 2"
    );
    assert_eq!(candidates["c001"]["startLine"], 8);
    assert!(asked[0]["questions"].get("q002").is_some());
}

#[tokio::test]
async fn a_test_body_batch_the_judge_finds_too_large_is_halved() {
    let fx = fixture(&[("test_needle.py", TESTS)], |_, _| {});
    let log = Log::default();
    let inner = python_judge(
        &log,
        |_| 0.9,
        TEST_ROLE,
        |name| if name == "test_keep" { 0.9 } else { 0.2 },
    );
    let evaluate: Evaluator = Arc::new(move |evaluation: Evaluation, deadline| {
        let evaluation = prompts::decoded(evaluation);
        let batch = evaluation
            .state
            .get("candidates")
            .map(|c| c.as_object().unwrap().len());
        if batch.is_some_and(|n| n > 1) {
            return Box::pin(async { Err(JudgeError::TooLarge) });
        }
        inner(evaluation, deadline)
    });
    let out = ask(&fx, None, evaluate).await;
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!(ranges(&out.files[0]), [(1, 7)]);
    let asked = sent(&log)
        .into_iter()
        .filter(|ev| ev["state"].get("candidates").is_some())
        .count();
    assert_eq!(asked, 3);
}

#[tokio::test]
async fn an_all_negative_test_body_pass_keeps_the_presentation() {
    let fx = fixture(&[("test_needle.py", TESTS)], |_, _| {});
    let out = ask(
        &fx,
        None,
        python_judge(&Log::default(), |_| 0.9, TEST_ROLE, |_| 0.1),
    )
    .await;
    assert_eq!(ranges(&out.files[0]), [(1, 14)]);
    // so does a file without the test role, which is never asked
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        python_judge(&log, |_| 0.9, &[("implementation", 0.9)], |_| 0.9),
    )
    .await;
    assert_eq!(ranges(&out.files[0]), [(1, 14)]);
    assert!(!log.lock().unwrap().join("").contains("candidates"));
}

#[tokio::test]
async fn a_large_python_file_is_previewed_by_sampled_ranges() {
    let filler: String = (0..800)
        .map(|i| format!("def filler_{i:03}(x):\n    return x + {i}\n\n\n"))
        .collect();
    let source = format!("{filler}def needle():\n    return 'found'\n\n\n{filler}");
    let fx = fixture(&[("needle.py", source.as_bytes())], |_, _| {});
    let log = Log::default();
    ask(&fx, None, judge(&log, keyword)).await;
    let previews: Vec<Value> = sent(&log)
        .into_iter()
        .filter_map(|ev| {
            ev["state"]["items"]
                .as_array()?
                .iter()
                .find(|item| item["path"] == "needle.py")
                .map(|item| item["filePreview"].clone())
        })
        .collect();
    assert_eq!(previews[0]["range"], "sampled source ranges");
    assert_eq!(previews[0]["truncated"], true);
    let text = previews[0]["text"].as_str().unwrap();
    assert!(text.contains("; query-named implementation ---\n    return 'found'\n"));
    assert!(text.len() <= 16_384);
}

#[tokio::test]
async fn shared_evidence_carries_a_selected_methods_neighbours() {
    let gap = "\n".repeat(10);
    let source = format!(
        "class Needle:\n    def a(self):\n        return 1\n{gap}\n    def b(self):\n        return 2\n{gap}\n    def c(self):\n        return 3\n"
    );
    let fx = fixture(&[("needle.py", source.as_bytes())], |_, _| {});
    let log = Log::default();
    let out = ask(
        &fx,
        None,
        python_judge(
            &log,
            |name| if name == "Needle.b" { 0.9 } else { 0.1 },
            &[],
            |_| 0.0,
        ),
    )
    .await;
    let shared: Vec<String> = sent(&log)
        .iter()
        .filter_map(|ev| ev["state"].get("selectedEvidence").map(Value::to_string))
        .collect();
    assert!(shared[0].contains("def c(self)"));
    // the presentation is not widened
    assert!(!texts(&out.files[0]).contains("def c(self)"));
}

#[tokio::test]
async fn a_spent_judge_token_budget_stops_the_ask_as_incomplete() {
    let files: &[(&str, &[u8])] = &[
        ("a/needle_dir/b/needle_dir/c/needle.rs", b"fn needle() {}\n"),
        ("a/other.rs", b"fn other() {}\n"),
    ];
    // The test judge bills 7 tokens a call; one slot keeps calls serial.
    let budgeted = fixture(files, |_, cfg| {
        cfg.find_relevant_judge_slots = 1;
        cfg.find_relevant_judge_token_budget = 14;
    });
    let log = Log::default();
    let out = ask(&budgeted, None, judge(&log, keyword)).await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("token_budget"));
    assert_eq!(out.issues.get("token_budget"), Some(&1));
    assert!(!out.issues.contains_key("deadline"), "{:?}", out.issues);
    assert_eq!(out.stats.judge_calls, 2);
    assert_eq!(out.stats.input_tokens, 14);
    assert_eq!(log.lock().unwrap().len(), 2);

    // 0 = unlimited: the same ask reaches the needle.
    let unlimited = fixture(files, |_, cfg| {
        cfg.find_relevant_judge_slots = 1;
        cfg.find_relevant_judge_token_budget = 0;
    });
    let out = ask(&unlimited, None, judge(&Log::default(), keyword)).await;
    assert!(out.stats.judge_calls > 2);
    assert!(!out.issues.contains_key("token_budget"));
    assert!(
        paths(&unlimited, &out)
            .iter()
            .any(|p| p.ends_with("needle.rs")),
        "{:?}",
        paths(&unlimited, &out)
    );
}
