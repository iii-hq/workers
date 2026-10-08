//! Engine-free asks over a tempdir with a recording fake judge.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use judge_contract::Evaluation;
use serde_json::Value;

use super::navigate::plan_batches;
use super::prompts::{key, FilePreview, Kind, NavigationItem};
use super::*;

// Fixture secrets below are split with concat! so push-time secret scanners
// do not flag them; the bytes the walk reads are unchanged.
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
        move |_| async move {
            Ok(Listing {
                window,
                models: None,
            })
        },
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
    assert_eq!(out.hint, None);
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
                concat!(
                    "-----BEGIN OPENSSH PRIVATE",
                    " KEY-----\nSECRET_PK needle\n"
                )
                .as_bytes(),
            ),
            (
                "src/core/key.asc",
                concat!(
                    "-----BEGIN PGP PRIVATE",
                    " KEY BLOCK-----\nSECRET_PGP needle\n"
                )
                .as_bytes(),
            ),
            (
                "src/core/putty.txt",
                concat!(
                    "PuTTY-User-Key",
                    "-File-3: ssh-ed25519\nSECRET_PPK needle\n"
                )
                .as_bytes(),
            ),
            (
                "src/core/age.txt",
                concat!(
                    "AGE-SECRET",
                    "-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ",
                    " SECRET_AGE needle\n"
                )
                .as_bytes(),
            ),
            ("deploy.ppk", b"SECRET_PPKNAME needle"),
            ("terraform.tfstate", b"{\"password\": \"SECRET_TF needle\"}"),
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
        "deploy.ppk",
        "terraform.tfstate",
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

#[test]
fn secret_keys_are_matched_by_shape_not_by_name() {
    let fx = fixture(
        &[
            (
                "age.txt",
                concat!(
                    "KEY=AGE-SECRET",
                    "-KEY-1QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7LQPZRY9X8GF2TVDW0S3JN54KHCE\n"
                )
                .as_bytes(),
            ),
            (
                "putty.txt",
                concat!("PuTTY-User-Key", "-File-2: ssh-rsa\n").as_bytes(),
            ),
            (
                "mentions.rs",
                concat!("// PuTTY-User-Key", "-File- and AGE-SECRET", "-KEY-1\n").as_bytes(),
            ),
        ],
        |_, _| {},
    );
    let tree = walk::Tree::new(&fx.resolver, &fx.root, None, &fx.root, u64::MAX);
    assert!(matches!(walk::read(&tree, "age.txt"), walk::Snap::Excluded));
    assert!(matches!(
        walk::read(&tree, "putty.txt"),
        walk::Snap::Excluded
    ));
    assert!(matches!(
        walk::read(&tree, "mentions.rs"),
        walk::Snap::Ok(_)
    ));
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
    let (batches, oversize) = plan_batches("q", tiny.clone(), usize::MAX, None);
    assert_eq!(oversize, 0);
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        [128, 128, 44]
    );
    let (batches, _) = plan_batches("q", tiny, navigate::MAX_REQUEST_BYTES, None);
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 300);
    for batch in &batches {
        let bytes = prompts::request_bytes(&prompts::navigation("q", batch));
        assert!(bytes <= navigate::MAX_REQUEST_BYTES, "{bytes}");
    }

    let mut big: Vec<_> = (0..7)
        .map(|i| file_item(&format!("f{i}.rs"), "a".repeat(10_000)))
        .collect();
    big.insert(3, file_item("huge.rs", "a".repeat(40_000)));
    let (batches, oversize) = plan_batches("q", big, navigate::MAX_REQUEST_BYTES, None);
    assert_eq!(oversize, 1, "a single item over the cap is dropped");
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [3, 3, 1]);
    for batch in &batches {
        let bytes = prompts::request_bytes(&prompts::navigation("q", batch));
        assert!(bytes <= navigate::MAX_REQUEST_BYTES, "{bytes}");
    }
    // a known window also caps a request at 2.5 bytes a token after its
    // questions' framing: two 10 000-byte items are over 8192 tokens
    let (batches, _) = plan_batches(
        "q",
        (0..4)
            .map(|i| file_item(&format!("f{i}.rs"), "a".repeat(10_000)))
            .collect(),
        navigate::MAX_REQUEST_BYTES,
        Some(8_192),
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

    // a single item the judge refuses is a request_size issue
    let log = Log::default();
    let out = ask(&fx, None, judge(&log, |_| Err(JudgeError::TooLarge))).await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.issues.get("request_size"), Some(&6));
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
    assert_eq!(
        out.hint.as_deref(),
        Some("No judge answered: use coder::search.")
    );
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
    assert!(out
        .hint
        .as_ref()
        .unwrap()
        .starts_with("Coverage is partial (deadline)"));
    assert!(out.issues.contains_key("deadline"));
    assert_eq!(paths(&fx, &out), ["needle.rs"]);
}

/// The judge fails every request that mentions `marker` with `error` and
/// scores the rest by [`keyword`].
fn failing_on(log: &Log, marker: &'static str, error: JudgeError) -> Evaluator {
    judge(log, move |ev| {
        if ev.state.to_string().contains(marker) {
            return Err(error.clone());
        }
        keyword(ev)
    })
}

#[tokio::test]
async fn a_call_the_judge_timed_out_is_its_own_issue_and_the_ask_goes_on() {
    // the later file sits a round below, in a request of its own
    let fx = fixture(
        &[
            ("needle.rs", b"needle"),
            ("needle_dir/sub/later.rs", b"needle failing"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let out = ask(&fx, None, failing_on(&log, "failing", JudgeError::Deadline)).await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("judge_call_timeout"));
    let hint = out.hint.as_deref().unwrap();
    assert!(hint.contains("files not listed"), "{hint}");
    assert!(hint.contains("coder::search"), "{hint}");
    assert!(hint.contains("larger timeout_ms"), "{hint}");
    assert!(
        out.issues.contains_key("judge_call_timeout"),
        "{:?}",
        out.issues
    );
    assert!(!out.issues.contains_key("deadline"), "{:?}", out.issues);
    assert_eq!(paths(&fx, &out), ["needle.rs"]);
}

#[tokio::test]
async fn an_evaluation_the_judge_failed_is_skipped_without_stopping_the_ask() {
    // the later file sits a round below, in a request of its own
    let fx = fixture(
        &[
            ("needle.rs", b"needle"),
            ("needle_dir/sub/later.rs", b"needle failing"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let out = ask(&fx, None, failing_on(&log, "failing", JudgeError::Invalid)).await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("invalid_response"));
    assert!(out.hint.as_ref().unwrap().ends_with("Retry the ask later."));
    assert!(
        out.issues.contains_key("invalid_response"),
        "{:?}",
        out.issues
    );
    assert!(!out.issues.contains_key("provider"), "{:?}", out.issues);
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
    let fx = fixture(&[("f.rs", b"x"), ("sub/g.rs", b"x")], |_, _| {});
    let log = Log::default();
    for (req, message) in [
        (input(" ", 120_000), "query must not be empty; retry with"),
        (input(&"q".repeat(4001), 120_000), "at most 4000: retry"),
        (
            input("q", 300_000),
            "timeout_ms is 300000; it must be within 1000..=280000: retry",
        ),
        (
            FindRelevantInput {
                path: "f.rs".into(),
                ..input("q", 120_000)
            },
            "not a directory: f.rs; find-relevant searches a folder: retry with path \".\"",
        ),
        (
            FindRelevantInput {
                path: "sub/g.rs".into(),
                ..input("q", 120_000)
            },
            "retry with path \"sub\", or read the file with coder::read-file",
        ),
    ] {
        let error = run(
            fx.resolver.clone(),
            fx.cfg.clone(),
            req,
            |_| async { Ok(Listing::default()) },
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
    let files = [
        ("a_needle.rs", source.as_bytes()),
        ("b_needle.rs", source.as_bytes()),
        ("c_needle.rs", source.as_bytes()),
    ];
    let answer = |ev: &Evaluation| {
        if ev.state.get("items").is_some() {
            keyword(ev)
        } else {
            Ok(ev.questions.keys().map(|k| (k.clone(), 0.9)).collect())
        }
    };
    let fx = fixture(&files, |_, _| {});
    let out = ask(&fx, None, judge(&Log::default(), answer)).await;
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

    // a lower `code.max_output_bytes` lowers the source budget: one fits
    let low = fixture(&files, |_, cfg| cfg.max_output_bytes = 60_000);
    let out = ask(&low, None, judge(&Log::default(), answer)).await;
    let omitted: Vec<bool> = out.files.iter().map(|f| f.source_omitted).collect();
    assert_eq!(omitted, [false, true, true]);
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
        hint: None,
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
    assert_eq!(many.reason.as_deref(), Some("resource_limit"));
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
            (
                ".git/config",
                concat!("url = https://user", ":TOKEN@host/repo\n").as_bytes(),
            ),
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
            |_| async { Ok(Listing::default()) },
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

/// `req`'s refusal, which no judge call precedes.
async fn refused(fx: &Fixture, req: FindRelevantInput) -> String {
    let log = Log::default();
    let error = run(
        fx.resolver.clone(),
        fx.cfg.clone(),
        req,
        |_| async { Ok(Listing::default()) },
        judge(&log, keyword),
        None,
    )
    .await
    .unwrap_err();
    assert!(log.lock().unwrap().is_empty());
    match error {
        CoderError::BadInput(message) => message,
        other => panic!("{other:?}"),
    }
}

fn at(path: &str) -> FindRelevantInput {
    FindRelevantInput {
        path: path.into(),
        ..input("where is the needle?", 120_000)
    }
}

#[tokio::test]
async fn a_gitignored_or_hidden_walk_root_is_refused() {
    let fx = fixture(
        &[
            (".git/HEAD", b"ref: refs/heads/main\n"),
            (".gitignore", b"/data/\nlogs\n"),
            ("data/conf/notes.toml", b"token = \"SECRET_IGN\" needle"),
            ("src/logs/needle.txt", b"SECRET_LOG needle"),
            (".tokens/hosts.yml", b"oauth_token: SECRET_DOT needle"),
            ("src/.cache/needle.rs", b"SECRET_CACHE needle"),
            ("src/needle.rs", b"fn needle() {}\n"),
            ("credentials/prod.json", b"SECRET_CRED needle"),
            // a repository nested in an ignored folder, and a planted `.git`
            ("data/lib/.git/HEAD", b"ref: refs/heads/main\n"),
            ("data/lib/conf.txt", b"SECRET_NESTED needle"),
            ("src/logs/.git", b"gitdir: x\n"),
            // a dot-folder whose `.git` file points nowhere
            (".planted/.git", b"gitdir: /nonexistent\n"),
            (".planted/needle.rs", b"SECRET_PLANTED needle"),
            // a dot-folder that is a repository of its own
            (".config/.git/HEAD", b"ref: refs/heads/main\n"),
            (".config/gh/hosts.yml", b"oauth_token: SECRET_CFG needle"),
        ],
        |_, _| {},
    );
    for path in ["data", "data/conf", "src/logs", "data/lib"] {
        let message = refused(&fx, at(path)).await;
        assert!(message.contains("gitignored"), "{path}: {message}");
        assert!(message.contains("coder::search"), "{path}: {message}");
    }
    for path in [
        ".tokens",
        "src/.cache",
        "credentials",
        ".config",
        ".config/gh",
        ".planted",
    ] {
        let message = refused(&fx, at(path)).await;
        assert!(
            message.contains("hidden or secret-named"),
            "{path}: {message}"
        );
    }
    let out = ask_with(&fx, at("src"), None, judge(&Log::default(), keyword)).await;
    assert_eq!(paths(&fx, &out), ["src/needle.rs"]);
}

/// Makes `<root>/<top>` a linked worktree of the repository at `root`, as
/// `git worktree add` does: a `.git` file naming an admin folder whose
/// `gitdir` points back at it.
fn link_worktree(root: &Path, top: &str) {
    let admin = root.join(".git/worktrees").join(top.replace('/', "-"));
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(root.join(".git/objects")).unwrap();
    let git = root.join(top).join(".git");
    std::fs::write(&git, format!("gitdir: {}\n", admin.display())).unwrap();
    std::fs::write(admin.join("gitdir"), format!("{}\n", git.display())).unwrap();
}

#[tokio::test]
async fn a_worktree_under_a_hidden_ignored_folder_is_searched() {
    use crate::fs::FsBoundary::Workspace;
    let fx = fixture(
        &[
            (".git/HEAD", b"ref: refs/heads/main\n"),
            (".gitignore", b".claude/\nwt/\n"),
            (".claude/worktrees/wt/src/needle.rs", b"fn needle() {}\n"),
            ("wt/feat/src/needle.rs", b"fn needle() {}\n"),
        ],
        |_, _| {},
    );
    link_worktree(&fx.root, ".claude/worktrees/wt");
    link_worktree(&fx.root, "wt/feat");
    for (top, req) in [
        (".claude/worktrees/wt", at(".claude/worktrees/wt")),
        (
            ".claude/worktrees/wt",
            scoped(".", &fx.root.join(".claude/worktrees/wt"), &[], Workspace),
        ),
        // a session on the main repository
        (
            ".claude/worktrees/wt",
            scoped(".claude/worktrees/wt", &fx.root, &[], Workspace),
        ),
        ("wt/feat", at("wt/feat")),
        ("wt/feat", scoped("wt/feat", &fx.root, &[], Workspace)),
    ] {
        let out = ask_with(&fx, req, None, judge(&Log::default(), keyword)).await;
        assert_eq!(paths(&fx, &out), [format!("{top}/src/needle.rs")]);
    }
}

#[tokio::test]
async fn ignore_files_apply_to_the_walk_root_outside_a_repository_too() {
    let fx = fixture(
        &[
            (".gitignore", b"/data/\n"),
            ("data/conf/notes.toml", b"token = \"SECRET_IGN\" needle"),
            ("src/needle.rs", b"fn needle() {}\n"),
        ],
        |_, _| {},
    );
    for path in ["data", "data/conf"] {
        let message = refused(&fx, at(path)).await;
        assert!(message.contains("gitignored"), "{path}: {message}");
    }
    let out = ask_with(&fx, at("src"), None, judge(&Log::default(), keyword)).await;
    assert_eq!(paths(&fx, &out), ["src/needle.rs"]);
}

#[tokio::test]
async fn ignore_files_above_the_repository_do_not_apply() {
    let fx = fixture(
        &[
            ("outer/.gitignore", b"*.rs\n"),
            ("outer/repo/.git/HEAD", b"ref: refs/heads/main\n"),
            ("outer/repo/src/needle.rs", b"fn needle() {}\n"),
        ],
        |_, _| {},
    );
    let out = ask_with(&fx, at("outer/repo"), None, judge(&Log::default(), keyword)).await;
    assert_eq!(paths(&fx, &out), ["outer/repo/src/needle.rs"]);
}

fn scoped(
    path: &str,
    root: &Path,
    grants: &[&Path],
    boundary: crate::fs::FsBoundary,
) -> FindRelevantInput {
    FindRelevantInput {
        fs_scope: Some(crate::fs::FsScope {
            root: root.display().to_string(),
            grants: grants.iter().map(|g| g.display().to_string()).collect(),
            boundary,
        }),
        ..at(path)
    }
}

#[tokio::test]
async fn an_unjailed_ask_needs_a_project_folder() {
    use crate::fs::FsBoundary::{ConfiguredRoots, Workspace};
    let fx = fixture(&[("needle.rs", b"fn needle() {}\n")], |_, cfg| {
        cfg.unjailed = true;
    });
    // not a dot-name like tempdir's own `.tmp…`
    let dir = tempfile::Builder::new().prefix("plain").tempdir().unwrap();
    let outside = dir.path().canonicalize().unwrap();
    std::fs::write(outside.join("needle.rs"), b"SECRET_HOST needle").unwrap();
    let wire = outside.display().to_string();
    // neither unscoped nor from a session elsewhere, and an unjailed
    // worker's roots (`/tmp`, its folder) only anchor relative paths
    for req in [
        at(&wire),
        scoped(&wire, &fx.root, &[], ConfiguredRoots),
        at("."),
    ] {
        let message = refused(&fx, req).await;
        assert!(message.contains("project folder"), "{message}");
    }
    // the session folder is one
    let out = ask_with(
        &fx,
        scoped(".", &fx.root, &[], Workspace),
        None,
        judge(&Log::default(), keyword),
    )
    .await;
    assert_eq!(paths(&fx, &out), ["needle.rs"]);
    // a granted dot-folder is still hidden
    let tokens = outside.join(".tokens");
    std::fs::create_dir(&tokens).unwrap();
    std::fs::write(
        tokens.join("hosts.yml"),
        b"oauth_token: SECRET_GRANT needle",
    )
    .unwrap();
    let req = scoped(
        &tokens.display().to_string(),
        &fx.root,
        &[&tokens],
        Workspace,
    );
    let message = refused(&granted(&fx, &req), req).await;
    assert!(message.contains("hidden"), "{message}");
    // a dot-folder repository found from the filesystem root is too
    std::fs::create_dir(tokens.join(".git")).unwrap();
    let message = refused(&fx, at(&tokens.display().to_string())).await;
    assert!(message.contains("hidden"), "{message}");
    // a Git work tree is a project folder
    std::fs::create_dir(outside.join(".git")).unwrap();
    let out = ask_with(&fx, at(&wire), None, judge(&Log::default(), keyword)).await;
    assert_eq!(out.files.len(), 1);
}

/// `req` against `fx` with the session's grants added to the resolver, as
/// the registered handler does.
fn granted(fx: &Fixture, req: &FindRelevantInput) -> Fixture {
    Fixture {
        resolver: fx.resolver.session_scoped(
            crate::fs::scope_root(req.fs_scope.as_ref()),
            crate::fs::scope_grants(req.fs_scope.as_ref()),
        ),
        cfg: fx.cfg.clone(),
        root: fx.root.clone(),
        _dir: tempfile::tempdir().unwrap(),
    }
}

#[tokio::test]
async fn a_repository_inside_a_dot_folder_is_searched_unjailed() {
    let fx = fixture(
        &[
            (".dotparent/repo/.git/HEAD", b"ref: refs/heads/main\n"),
            (".dotparent/repo/needle.rs", b"fn needle() {}\n"),
        ],
        |_, cfg| cfg.unjailed = true,
    );
    let dotparent = fx.root.join(".dotparent");
    let repo = dotparent.join("repo").display().to_string();
    let out = ask_with(&fx, at(&repo), None, judge(&Log::default(), keyword)).await;
    assert_eq!(paths(&fx, &out), [".dotparent/repo/needle.rs"]);
    // the dot-folder itself, granted, is still hidden
    let req = scoped(
        &dotparent.display().to_string(),
        &fx.root,
        &[&dotparent],
        crate::fs::FsBoundary::Workspace,
    );
    let message = refused(&granted(&fx, &req), req).await;
    assert!(message.contains("hidden"), "{message}");
}

#[tokio::test]
async fn exclude_globs_match_from_the_work_tree_without_a_session() {
    let fx = fixture(&[], |_, cfg| cfg.unjailed = true);
    // outside every configured root, not a dot-name like tempdir's own
    let dir = tempfile::Builder::new().prefix("plain").tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap();
    for path in [".git/HEAD", "ade/needle.rs", "ade/gen/needle.rs"] {
        std::fs::create_dir_all(repo.join(path).parent().unwrap()).unwrap();
        std::fs::write(repo.join(path), b"fn needle() {}\n").unwrap();
    }
    let req = FindRelevantInput {
        exclude_globs: vec!["ade/gen/**".into()],
        ..at(&repo.join("ade").display().to_string())
    };
    let out = ask_with(&fx, req, None, judge(&Log::default(), keyword)).await;
    assert_eq!(
        out.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
        [repo.join("ade/needle.rs").display().to_string()]
    );
}

#[tokio::test]
async fn agents_md_stops_at_a_grant_outside_the_session() {
    let fx = fixture(
        &[
            (".git/HEAD", b"ref: refs/heads/main\n"),
            ("AGENTS.md", b"top rules"),
            ("session/a.rs", b"x"),
            ("grant/AGENTS.md", b"grant rules"),
            ("grant/needle.rs", b"fn needle() {}\n"),
        ],
        |_, _| {},
    );
    let grant = fx.root.join("grant");
    let req = scoped(
        &grant.display().to_string(),
        &fx.root.join("session"),
        &[&grant],
        crate::fs::FsBoundary::Workspace,
    );
    let out = ask_with(
        &granted(&fx, &req),
        req,
        None,
        judge(&Log::default(), keyword),
    )
    .await;
    assert_eq!(paths(&fx, &out), ["grant/needle.rs"]);
    assert_eq!(
        out.agents_md,
        [grant.join("AGENTS.md").display().to_string()]
    );
}

#[tokio::test]
async fn exclude_globs_match_from_the_session_root_like_coder_search() {
    let fx = fixture(
        &[
            ("sub/needle.rs", b"fn needle() {}\n"),
            ("sub/gen/needle.rs", b"fn needle() {}\n"),
        ],
        |_, _| {},
    );
    let log = Log::default();
    let req = FindRelevantInput {
        exclude_globs: vec!["sub/gen/**".into()],
        ..at("sub")
    };
    let out = ask_with(&fx, req, None, judge(&log, keyword)).await;
    assert_eq!(paths(&fx, &out), ["sub/needle.rs"]);
    let sent = log.lock().unwrap().join("\n");
    for forbidden in ["\"gen", "\\\"gen"] {
        assert!(!sent.contains(forbidden), "{forbidden} reached the judge");
    }
    let search: crate::code::functions::search::SearchInput =
        serde_json::from_value(serde_json::json!({
            "query": "needle",
            "path": "sub",
            "exclude_globs": ["sub/gen/**"],
        }))
        .unwrap();
    let found = crate::code::functions::search::handle(fx.resolver.clone(), fx.cfg.clone(), search)
        .await
        .unwrap();
    let searched: Vec<_> = found
        .content_matches
        .iter()
        .map(|m| m.path.clone())
        .collect();
    assert_eq!(
        searched,
        [fx.root.join("sub/needle.rs").display().to_string()]
    );
}

#[tokio::test]
async fn a_preview_too_big_for_the_window_is_scored_in_chunks_keeping_the_best() {
    // 375 lines of 80 bytes: three 12 000-byte chunks, `alpha` in the first,
    // `needle` in the last; 30 quotes a line escape to 110 bytes of JSON
    let line = |word: &str| format!("{word:<49}{}\n", "\"".repeat(30));
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
    // the 22 528-byte preview is over 8192 tokens at 2.5 bytes a token, too
    // big to assess
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(
        out.issues,
        BTreeMap::from([("request_size".to_string(), 1)])
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
async fn a_large_file_fits_a_small_window_whole_and_is_assessed() {
    // ~175 KB and 200 declarations: the preview's declaration index alone
    // would be over 32 000 bytes
    let mut source = String::from("// needle\n");
    for i in 0..200 {
        source.push_str(&format!(
            "fn handle_filesystem_request_number_{i:03}_with_a_long_descriptive_name() {{\n"
        ));
        for j in 0..36 {
            source.push_str(&format!("    let value_{j:02} = {j};\n"));
        }
        source.push_str("}\n");
    }
    assert!(
        (170_000..180_000).contains(&source.len()),
        "{}",
        source.len()
    );
    let fx = fixture(&[("big.rs", source.as_bytes())], |_, _| {});
    for window in [16_384, 10_000] {
        let log = Log::default();
        let inner = judge(&log, keyword);
        let over = Arc::new(AtomicUsize::new(0));
        let counted = over.clone();
        let evaluate: Evaluator = Arc::new(move |evaluation, deadline| {
            if !prompts::fits(&evaluation, usize::MAX, Some(window)) {
                counted.fetch_add(1, Ordering::SeqCst);
            }
            inner(evaluation, deadline)
        });
        let out = ask(&fx, Some(window), evaluate).await;
        assert_eq!(over.load(Ordering::SeqCst), 0, "window {window}");
        assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
        assert_eq!(paths(&fx, &out), ["big.rs"]);
        assert!(out.files[0].priority.is_some());
        let log = log.lock().unwrap();
        let navigation: Vec<_> = log.iter().filter(|s| s.contains("\"items\"")).collect();
        // one preview item, not chunks, with part of its index
        assert_eq!(navigation.len(), 1);
        assert!(!navigation[0].contains("sampled source ranges"));
        assert!(navigation[0].contains("\"declarationIndexTruncated\":true"));
        assert!(navigation[0].contains("number_000"));
    }
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
        |_| async { Err(JudgeError::Unavailable(judge::LISTING_TIMEOUT.into())) },
        judge(&log, keyword),
        None,
    )
    .await
    .unwrap();
    assert_eq!(out.status, Status::Unavailable);
    assert_eq!(out.reason.as_deref(), Some(judge::LISTING_TIMEOUT));
    assert!(out.hint.unwrap().contains("retry the ask in a minute"));
    assert!(log.lock().unwrap().is_empty());

    // a paused judge: fall back now, retry after the pause
    let out = ask(&fx, None, judge(&log, |_| Err(JudgeError::Paused))).await;
    assert_eq!(out.status, Status::Unavailable);
    assert_eq!(out.reason.as_deref(), Some(judge::PAUSED));
    let hint = out.hint.unwrap();
    assert!(hint.contains("paused for up to 30 s"), "{hint}");
    assert!(hint.contains("use coder::search"), "{hint}");
}

#[tokio::test]
async fn a_complete_result_without_files_hints_to_widen_the_path() {
    let fx = fixture(
        &[(
            "src/other.rs",
            b"fn other() {}
",
        )],
        |_, _| {},
    );
    let out = ask(&fx, None, judge(&Log::default(), keyword)).await;
    assert_eq!(out.status, Status::Complete);
    assert!(out.files.is_empty());
    let hint = out.hint.unwrap();
    assert!(hint.contains("widen path"), "{hint}");
    assert!(hint.contains("coder::search"), "{hint}");
}

#[cfg(unix)]
#[tokio::test]
async fn an_unlistable_folder_above_the_path_only_marks_agents_md() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture(&[("outer/src/needle.rs", b"fn needle() {}\n")], |_, _| {});
    let outer = fx.root.join("outer");
    // traversable, not listable
    std::fs::set_permissions(&outer, std::fs::Permissions::from_mode(0o111)).unwrap();
    let listable = std::fs::read_dir(&outer).is_ok(); // root ignores modes
    let out = ask_with(&fx, at("outer/src"), None, judge(&Log::default(), keyword)).await;
    std::fs::set_permissions(&outer, std::fs::Permissions::from_mode(0o755)).unwrap();
    if listable {
        return;
    }
    assert_eq!(out.issues.get("agents_md_incomplete"), Some(&1));
    assert_eq!(out.status, Status::Complete, "{:?}", out.issues);
    assert_eq!((&out.reason, &out.hint), (&None, &None));
    assert_eq!(paths(&fx, &out), ["outer/src/needle.rs"]);
}

#[tokio::test]
async fn a_missing_path_names_the_closest_eligible_folders_beside_it() {
    let fx = fixture(
        &[
            (".git/HEAD", b"ref: refs/heads/main\n"),
            (".gitignore", b"judge-ignored/\n"),
            ("judge/a.rs", b"x"),
            ("judge-clef/src/a.rs", b"x"),
            ("judge-typesafe/a.rs", b"x"),
            ("judge-denied/a.rs", b"x"),
            ("judge-ignored/a.rs", b"x"),
            ("judge-ignored/out/a.rs", b"x"),
            (".judge-hub/a.rs", b"x"),
            (".github/workflows/ci.yml", b"x"),
            ("harness/a.rs", b"x"),
            ("ide/a.rs", b"x"),
            ("console/a.rs", b"x"),
            ("judge-hub.rs", b"x"),
        ],
        |root, cfg| cfg.denylist_paths = vec![root.join("judge-denied")],
    );
    let refusal = |path: &'static str| {
        let fx = &fx;
        async move {
            run(
                fx.resolver.clone(),
                fx.cfg.clone(),
                at(path),
                |_| async { Ok(Listing::default()) },
                judge(&Log::default(), keyword),
                None,
            )
            .await
            .unwrap_err()
        }
    };
    let error = refusal("judge-hub").await;
    assert_eq!(error.code(), "C211");
    assert_eq!(
        error.message(),
        "judge-hub: not found or not accessible. Verify the path with coder::list-folder \
         or coder::tree. Folders beside it, closest name first: judge, judge-clef, \
         ide, harness, judge-typesafe."
    );
    // a denied folder reads like a missing one: same list, never itself
    let denied = refusal("judge-denied").await;
    let missing = refusal("judge-dented").await;
    assert_eq!(denied.code(), "C211");
    assert_eq!(
        denied.message().replace("judge-denied", "X"),
        missing.message().replace("judge-dented", "X")
    );
    assert!(!missing.message().contains("judge-denied"));
    assert!(!missing.message().contains("judge-ignored"));
    // a nested path names folders under its parent; none, none named
    let nested = refusal("judge-clef/sr").await;
    assert!(
        nested.message().ends_with("first: judge-clef/src."),
        "{}",
        nested.message()
    );
    let bare = refusal("judge/sub").await;
    assert_eq!(
        bare.message(),
        CoderError::not_found_or_denied("judge/sub").message()
    );
    // a parent the ask itself would refuse (hidden, ignored) names none
    for path in [".github/workflowz", "judge-ignored/ou"] {
        assert_eq!(
            refusal(path).await.message(),
            CoderError::not_found_or_denied(path).message()
        );
    }
}

#[tokio::test]
async fn a_missing_relative_path_on_an_unjailed_worker_names_its_anchor() {
    let fx = fixture(
        &[
            (".git/HEAD", b"ref: refs/heads/main\n"),
            ("judge/a.rs", b"x"),
        ],
        |_, cfg| cfg.unjailed = true,
    );
    let error = run(
        fx.resolver.clone(),
        fx.cfg.clone(),
        at("judge-hub"),
        |_| async { Ok(Listing::default()) },
        judge(&Log::default(), keyword),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "C211");
    assert_eq!(
        error.message(),
        format!(
            "judge-hub: not found or not accessible. Verify the path with \
             coder::list-folder or coder::tree. Relative paths resolve against {}; \
             pass an absolute path.",
            fx.root.display()
        )
    );
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
    let tree = walk::Tree::new(&fx.resolver, &fx.root, None, &fx.root, u64::MAX);
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

/// Navigation by [`keyword`]; evidence by `answer(name)` with scope 1;
/// anything else 0.1.
fn by_declaration(ev: &Evaluation, answer: impl Fn(&str) -> f64) -> Result<Scores, JudgeError> {
    let Some(declarations) = ev.state["declarations"].as_array() else {
        return keyword(ev);
    };
    let mut scores = Scores::new();
    for (i, d) in declarations.iter().enumerate() {
        scores.insert(key("q", i), answer(d["name"].as_str().unwrap()));
        scores.insert(key("scope", i), 1.0);
    }
    Ok(scores)
}

const TWO_FUNCTIONS: &[u8] =
    b"fn needle() -> u32 {\n    helper()\n}\n\n\n\n\n\n\n\nfn helper() -> u32 {\n    7\n}\n";

fn texts(file: &RelevantFile) -> String {
    file.excerpts.iter().map(|e| e.text.as_str()).collect()
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
                return by_declaration(ev, |name| if name == "needle_a" { 0.9 } else { 0.6 });
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
async fn a_reply_missing_an_answer_is_an_invalid_response() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            let mut scores = by_declaration(ev, |_| 0.9)?;
            scores.remove("scope000");
            Ok(scores)
        }),
    )
    .await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("invalid_response"));
    assert_eq!(out.issues.get("provider"), Some(&1));
}

#[tokio::test]
async fn answers_are_reused_across_asks_of_one_provider_and_model() {
    let fx = fixture(&[("needle.rs", TWO_FUNCTIONS)], |_, _| {});
    // `fail` rejects the file assessment, which must not be cached
    let judge_for = |log: &Log, fail: bool| {
        judge(log, move |ev| {
            if fail && ev.questions.contains_key("priority") {
                return Err(JudgeError::Rejected("invalid_request".into()));
            }
            by_declaration(ev, |name| if name == "needle" { 0.9 } else { 0.1 })
        })
    };
    let provider = format!("test-{}", std::process::id());
    let ask_listing = |log: &Log, fail: bool, models: Option<&[&str]>| {
        let models = models.map(|names| names.iter().map(|n| n.to_string()).collect());
        run(
            fx.resolver.clone(),
            fx.cfg.clone(),
            input("where is the needle?", 120_000),
            |_| async {
                Ok(Listing {
                    window: None,
                    models,
                })
            },
            judge_for(log, fail),
            Some(provider.clone()),
        )
    };
    let ask_cached = |log: &Log, fail: bool| ask_listing(log, fail, Some(&["m"]));
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

    // another model behind the provider, or a listing that failed, asks
    // the judge again
    for models in [Some(&["m2"][..]), None] {
        let out = ask_listing(&Log::default(), false, models).await.unwrap();
        assert_eq!(out.stats.cache_hits, 0, "{models:?}");
        assert_eq!(out.stats.judge_calls, first.stats.judge_calls, "{models:?}");
    }
    // a named provider keeps its answers by name while its listing fails
    let again = ask_listing(&Log::default(), false, None).await.unwrap();
    assert_eq!(again.stats.judge_calls, 0);
    assert_eq!(again.stats.cache_hits, first.stats.judge_calls);
    // the hub default may have switched unseen: never cached
    for _ in 0..2 {
        let out = run(
            fx.resolver.clone(),
            fx.cfg.clone(),
            input("where is the needle?", 120_000),
            |_| async { Ok(Listing::default()) },
            judge_for(&Log::default(), false),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.stats.cache_hits, 0);
    }
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
    // a narrowed path still lists the ones above it, through the same gates
    let out = ask_with(&fx, at("src/core"), None, judge(&log, keyword)).await;
    assert_eq!(
        out.agents_md,
        [
            format!("{root}/AGENTS.md"),
            format!("{root}/src/core/AGENTS.md")
        ]
    );
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
                return by_declaration(ev, |name| if name == "needle" { 0.9 } else { 0.1 });
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

fn ranges(file: &RelevantFile) -> Vec<(u32, u32)> {
    file.excerpts
        .iter()
        .map(|e| (e.line_from, e.line_to))
        .collect()
}

const CALLER: &[u8] = b"class Base:\n    def run(self):\n        return 0\n\n\n\n\n\n\n\n\n\n\nclass Needle(Base):\n    def go(self):\n        return self.run()\n";

#[tokio::test]
async fn a_selected_python_method_shows_the_local_method_it_calls() {
    let fx = fixture(&[("needle.py", CALLER)], |_, _| {});
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            by_declaration(ev, |name| if name == "Needle.go" { 0.9 } else { 0.1 })
        }),
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

#[tokio::test]
async fn local_call_context_is_presented_after_a_judge_outage_stop() {
    let fx = fixture(&[("needle.py", CALLER)], |_, _| {});
    // the file assessment fails last, after the evidence was selected
    let evaluate: Evaluator = Arc::new(|evaluation, _| {
        let evaluation = prompts::decoded(evaluation);
        Box::pin(async move {
            if evaluation.questions.contains_key("priority") {
                tokio::time::sleep(Duration::from_millis(200)).await;
                return Err(JudgeError::Unavailable("transport".into()));
            }
            by_declaration(
                &evaluation,
                |name| if name == "Needle.go" { 0.9 } else { 0.1 },
            )
            .map(|scores| (scores, 7))
        })
    });
    let out = ask(&fx, None, evaluate).await;
    assert_eq!(out.status, Status::Incomplete);
    assert_eq!(out.reason.as_deref(), Some("transport"));
    let file = &out.files[0];
    assert!(file.roles.is_empty());
    assert_eq!(file.call_leads.len(), 1, "{file:?}");
    assert_eq!(file.call_leads[0].name, "Base.run");
}

#[tokio::test]
async fn a_test_file_shows_all_its_selection_after_a_token_budget_stop() {
    let source = b"fn needle_a() {}\n\n\n\n\n\n\n\n\nfn needle_b() {}\n";
    // navigation and the assessment spend 14 of 15 tokens; the evidence
    // call lands last and spends the budget before the presentation
    let fx = fixture(&[("t_needle.rs", source)], |_, cfg| {
        cfg.find_relevant_judge_token_budget = 15;
    });
    let out = ask(
        &fx,
        None,
        judge(&Log::default(), |ev| {
            if !ev.questions.contains_key("priority") {
                // needle_b is selected (0.6) but not presented (≤ 0.7)
                return by_declaration(ev, |name| if name == "needle_a" { 0.9 } else { 0.6 });
            }
            Ok(ev
                .questions
                .keys()
                .map(|id| (id.clone(), if id == "test" { 0.9 } else { 0.1 }))
                .collect())
        }),
    )
    .await;
    assert_eq!(out.reason.as_deref(), Some("token_budget"));
    assert_eq!(out.stats.judge_calls, 3);
    assert_eq!(out.files[0].roles, ["test"]);
    assert!(texts(&out.files[0]).contains("fn needle_b"));
}
