//! GOLDEN FAMILY B — error-message format snapshots for every C2xx shape.
//!
//! Each case constructs its error through the REAL public paths
//! (`PathResolver` construction/resolution + the per-verb handlers) in a
//! fixed two-root temp layout, then normalizes machine-specific path
//! prefixes (the canonical tempdir roots become `<ROOT0>` / `<ROOT1>`)
//! before comparing against `tests/golden/errors.json` (map: case name ->
//! `{code, message}`).
//!
//! Pinned shapes (T3's prescriptive messages + redaction invariant):
//! - C210: both-set root config; bad create-file mode; move cross-root
//!   directory; move destination-is-a-directory (prescriptive target hint);
//!   undefined replacement capture reference (v0.4.1 pre-write guard —
//!   the production silent-corruption repro from session q8x6g248)
//! - C211: missing path; glob-denied path (byte-identical suffix —
//!   REDACTION INVARIANT); recursive-delete subtree-blocked; move missing
//!   source
//! - C213: read cap; write cap; batch budget exhausted; full-read output
//!   budget (S4's recovery-tool message: size + total_lines + corrective
//!   calls)
//! - C215: relative `..` escape; absolute outside all roots; dangling
//!   symlink
//! - C216: io passthrough (via the public `From<io::Error>` conversion —
//!   see the case comment for why no handler drives this one)
//! - C217: create-file exists without overwrite; move destination exists
//!   without overwrite (one house C217 shape)
//!
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use coder::config::CoderConfig;
use coder::error::CoderError;
use coder::functions::{create_file, delete_file, move_file, read_file, update_file};
use coder::path::PathResolver;

/// Normalized `{code, message}` pair as written to the golden file.
#[derive(serde::Serialize)]
struct GoldenError {
    code: String,
    message: String,
}

/// Fixed two-root jail for the error cases. `<ROOT0>` is primary.
struct Jail {
    _tmp0: tempfile::TempDir,
    _tmp1: tempfile::TempDir,
    root0: PathBuf,
    root1: PathBuf,
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
}

impl Jail {
    fn new() -> Self {
        let tmp0 = tempfile::tempdir().unwrap();
        let tmp1 = tempfile::tempdir().unwrap();
        // cfg.base_paths holds the RAW tempdir forms; root0/root1 below are
        // the CANONICAL forms the error messages name (PathResolver
        // canonicalizes internally — on macOS the two differ: /var vs
        // /private/var).
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp0.path().to_path_buf(), tmp1.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        let root0 = std::fs::canonicalize(tmp0.path()).unwrap();
        let root1 = std::fs::canonicalize(tmp1.path()).unwrap();
        Self {
            _tmp0: tmp0,
            _tmp1: tmp1,
            root0,
            root1,
            resolver,
            cfg,
        }
    }

    /// Replace the canonical tempdir roots with stable tokens so the
    /// golden file is machine-independent. Longest root first in case one
    /// string is a prefix of the other.
    fn normalize(&self, msg: &str) -> String {
        let mut subs = [
            (self.root0.display().to_string(), "<ROOT0>"),
            (self.root1.display().to_string(), "<ROOT1>"),
        ];
        subs.sort_by_key(|(raw, _)| std::cmp::Reverse(raw.len()));
        let mut out = msg.to_string();
        for (raw, token) in subs {
            out = out.replace(&raw, token);
        }
        out
    }
}

/// Parse a top-level handler `Err(String)` — the wire JSON
/// `{"code":"C2xx","message":"..."}` — into (code, message).
fn parse_wire_string(err: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(err)
        .unwrap_or_else(|e| panic!("handler error is not wire JSON ({e}): {err}"));
    (
        v["code"].as_str().expect("code").to_string(),
        v["message"].as_str().expect("message").to_string(),
    )
}

fn from_coder_error(e: &CoderError) -> (String, String) {
    (e.code().to_string(), e.message().to_string())
}

async fn read_err(jail: &Jail, path: &str) -> (String, String) {
    let err = read_file::handle(
        jail.resolver.clone(),
        jail.cfg.clone(),
        read_file::ReadFileInput {
            path: Some(path.into()),
            ..read_file::ReadFileInput::default()
        },
    )
    .await
    .expect_err("read must fail");
    parse_wire_string(&err)
}

async fn create_err(
    jail: &Jail,
    cfg: Arc<CoderConfig>,
    spec: create_file::CreateFileSpec,
) -> (String, String) {
    let out = create_file::handle(
        jail.resolver.clone(),
        cfg,
        create_file::CreateFileInput { files: vec![spec] },
    )
    .await
    .expect("create-file batches never fail top-level for per-entry errors");
    let wire = out.results[0]
        .error
        .as_ref()
        .expect("entry must carry an error");
    (wire.code.clone(), wire.message.clone())
}

#[tokio::test]
async fn error_message_formats_match_golden() {
    let jail = Jail::new();
    let mut cases: BTreeMap<String, GoldenError> = BTreeMap::new();
    let mut put = |name: &str, (code, message): (String, String), jail: &Jail| {
        cases.insert(
            name.to_string(),
            GoldenError {
                code,
                message: jail.normalize(&message),
            },
        );
    };

    // --- C210: operator config error — both root forms set -------------
    {
        let cfg = CoderConfig {
            base_path: Some(jail.root0.clone()),
            base_paths: vec![jail.root1.clone()],
            ..CoderConfig::default()
        };
        let err = PathResolver::new(&cfg).expect_err("both-set must fail");
        put(
            "C210_config_both_root_forms_set",
            from_coder_error(&err),
            &jail,
        );
    }

    // --- C210: bad input — malformed create-file mode ------------------
    {
        let got = create_err(
            &jail,
            jail.cfg.clone(),
            create_file::CreateFileSpec {
                path: "bad-mode.txt".into(),
                content: "x".into(),
                mode: "9z9".into(),
                parents: true,
                overwrite: false,
            },
        )
        .await;
        put("C210_create_bad_mode", got, &jail);
    }

    // --- C211: missing vs glob-denied (REDACTION INVARIANT) ------------
    std::fs::write(jail.root0.join(".env"), "secret").unwrap();
    let missing = read_err(&jail, "missing.txt").await;
    let denied = read_err(&jail, ".env").await;
    {
        // Byte-identical suffix after the caller-supplied path prefix:
        // callers must not be able to distinguish "missing" from "denied".
        let m_suffix = missing
            .1
            .strip_prefix("missing.txt: ")
            .expect("missing message starts with its wire path");
        let d_suffix = denied
            .1
            .strip_prefix(".env: ")
            .expect("denied message starts with its wire path");
        assert_eq!(
            m_suffix, d_suffix,
            "C211 missing vs glob-denied suffixes must be byte-identical"
        );
    }
    put("C211_read_missing", missing, &jail);
    put("C211_read_glob_denied", denied, &jail);

    // --- C211: recursive delete refused on non-accessible subtree ------
    {
        std::fs::create_dir_all(jail.root0.join("blocked-dir/nested")).unwrap();
        std::fs::write(jail.root0.join("blocked-dir/nested/.env"), "s").unwrap();
        let out = delete_file::handle(
            jail.resolver.clone(),
            delete_file::DeleteFileInput {
                paths: vec!["blocked-dir".into()],
                recursive: true,
            },
        )
        .await
        .unwrap();
        let wire = out.results[0].error.as_ref().expect("subtree must block");
        put(
            "C211_delete_subtree_blocked",
            (wire.code.clone(), wire.message.clone()),
            &jail,
        );
    }

    // --- C213: read cap and write cap -----------------------------------
    {
        let tiny = Arc::new(CoderConfig {
            base_paths: vec![jail.root0.clone(), jail.root1.clone()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 8,
            max_write_bytes: 8,
            ..CoderConfig::default()
        });
        std::fs::write(jail.root0.join("big.txt"), "0123456789ABCDEF").unwrap();
        let err = read_file::handle(
            jail.resolver.clone(),
            tiny.clone(),
            read_file::ReadFileInput {
                path: Some("big.txt".into()),
                ..read_file::ReadFileInput::default()
            },
        )
        .await
        .expect_err("over-cap read must fail");
        put("C213_read_cap_exceeded", parse_wire_string(&err), &jail);

        let got = create_err(
            &jail,
            tiny,
            create_file::CreateFileSpec {
                path: "big-create.txt".into(),
                content: "0123456789ABCDEF".into(),
                mode: "0644".into(),
                parents: true,
                overwrite: false,
            },
        )
        .await;
        put("C213_write_cap_exceeded", got, &jail);
    }

    // --- C213: batch budget exhausted -------------------------------------
    {
        let tiny_budget = Arc::new(CoderConfig {
            base_paths: vec![jail.root0.clone(), jail.root1.clone()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            batch_read_budget_bytes: 5,
            ..CoderConfig::default()
        });
        std::fs::write(jail.root0.join("budget-a.txt"), "abcde").unwrap();
        std::fs::write(jail.root0.join("budget-b.txt"), "next").unwrap();
        let out = read_file::handle(
            jail.resolver.clone(),
            tiny_budget,
            read_file::ReadFileInput {
                paths: Some(vec![
                    read_file::ReadTarget::Path("budget-a.txt".into()),
                    read_file::ReadTarget::Path("budget-b.txt".into()),
                ]),
                ..read_file::ReadFileInput::default()
            },
        )
        .await
        .expect("batch never fails top-level");
        let results = out.results.unwrap();
        let wire = results[1].error.as_ref().expect("second entry must fail");
        put(
            "C213_batch_budget_exhausted",
            (wire.code.clone(), wire.message.clone()),
            &jail,
        );
    }

    // --- C213: full-read output budget (the recovery-tool message) -------
    {
        let tiny_output = Arc::new(CoderConfig {
            base_paths: vec![jail.root0.clone(), jail.root1.clone()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_output_bytes: 8,
            ..CoderConfig::default()
        });
        std::fs::write(jail.root0.join("over-output.txt"), "L1\nL2\nL3\n").unwrap();
        let err = read_file::handle(
            jail.resolver.clone(),
            tiny_output,
            read_file::ReadFileInput {
                path: Some("over-output.txt".into()),
                ..read_file::ReadFileInput::default()
            },
        )
        .await
        .expect_err("over-output full read must fail");
        put(
            "C213_full_read_output_budget_exceeded",
            parse_wire_string(&err),
            &jail,
        );
    }

    // --- C215: relative `..` escape / absolute outside / dangling link --
    put(
        "C215_relative_dotdot_escape",
        read_err(&jail, "../escape.txt").await,
        &jail,
    );
    put(
        "C215_absolute_outside_all_roots",
        read_err(&jail, "/etc/passwd").await,
        &jail,
    );
    {
        std::os::unix::fs::symlink(jail.root0.join("missing-target"), jail.root0.join("dangle"))
            .unwrap();
        put(
            "C215_dangling_symlink",
            read_err(&jail, "dangle/child.txt").await,
            &jail,
        );
    }

    // --- C216: io passthrough -------------------------------------------
    // No handler drives this one deterministically: a real EACCES needs a
    // chmod-000 directory, which silently stops failing when tests run as
    // root (CI containers). The public `From<io::Error>` conversion in
    // error.rs IS the path every handler's `?` takes for non-NotFound io
    // errors, so pinning it pins the C216 wire shape: the io error text
    // passes through verbatim under code C216.
    {
        let e = CoderError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            "synthetic io failure",
        ));
        put("C216_io_passthrough", from_coder_error(&e), &jail);
    }

    // --- C217: create-file exists without overwrite ---------------------
    {
        std::fs::write(jail.root0.join("exists.txt"), "old").unwrap();
        let got = create_err(
            &jail,
            jail.cfg.clone(),
            create_file::CreateFileSpec {
                path: "exists.txt".into(),
                content: "new".into(),
                mode: "0644".into(),
                parents: true,
                overwrite: false,
            },
        )
        .await;
        put("C217_create_exists_without_overwrite", got, &jail);
    }

    // --- C217: move destination exists without overwrite ----------------
    {
        std::fs::write(jail.root0.join("move-src.txt"), "src").unwrap();
        std::fs::write(jail.root0.join("move-dst.txt"), "dst").unwrap();
        let out = move_file::handle(
            jail.resolver.clone(),
            move_file::MoveFileInput {
                files: vec![move_file::MoveFileSpec {
                    from: "move-src.txt".into(),
                    to: "move-dst.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
            },
        )
        .await
        .expect("move batches never fail top-level");
        let wire = out.results[0]
            .error
            .as_ref()
            .expect("entry must carry an error");
        put(
            "C217_move_dst_exists_without_overwrite",
            (wire.code.clone(), wire.message.clone()),
            &jail,
        );
    }

    // --- C210: move — cross-root directory move rejected ----------------
    {
        std::fs::create_dir_all(jail.root0.join("cross-dir")).unwrap();
        std::fs::write(jail.root0.join("cross-dir/f.txt"), "x").unwrap();
        let dst_abs = jail.root1.join("cross-dir");
        let out = move_file::handle(
            jail.resolver.clone(),
            move_file::MoveFileInput {
                files: vec![move_file::MoveFileSpec {
                    from: "cross-dir".into(),
                    to: dst_abs.display().to_string(),
                    overwrite: false,
                    parents: true,
                }],
            },
        )
        .await
        .expect("move batches never fail top-level");
        let wire = out.results[0]
            .error
            .as_ref()
            .expect("entry must carry an error");
        put(
            "C210_move_cross_root_dir",
            (wire.code.clone(), jail.normalize(&wire.message)),
            &jail,
        );
    }

    // --- C210: move — destination is a directory (prescriptive) ---------
    {
        std::fs::write(jail.root0.join("move-dir-src.txt"), "x").unwrap();
        std::fs::create_dir_all(jail.root0.join("move-dst-dir")).unwrap();
        let out = move_file::handle(
            jail.resolver.clone(),
            move_file::MoveFileInput {
                files: vec![move_file::MoveFileSpec {
                    from: "move-dir-src.txt".into(),
                    to: "move-dst-dir".into(),
                    overwrite: false,
                    parents: true,
                }],
            },
        )
        .await
        .expect("move batches never fail top-level");
        let wire = out.results[0]
            .error
            .as_ref()
            .expect("entry must carry an error");
        put(
            "C210_move_dst_is_directory",
            (wire.code.clone(), wire.message.clone()),
            &jail,
        );
    }

    // --- C210: update-file — undefined replacement capture reference ----
    // v0.4.1 pre-write guard, pinned with the EXACT production repro
    // (session q8x6g248): a replacement carrying a JS template literal
    // (`Hello, ${name}!`) against a pattern with no capture groups. The
    // old behavior silently expanded `${name}` to the empty string and
    // wrote the corrupted file with success: true.
    {
        std::fs::write(
            jail.root0.join("tpl-handler.js"),
            "iii.registerFunction({ handler: () => 'hi' });\n",
        )
        .unwrap();
        let out =
            update_file::handle(
                jail.resolver.clone(),
                jail.cfg.clone(),
                update_file::UpdateFileInput {
                    files: vec![update_file::UpdateFileSpec {
                        path: "tpl-handler.js".into(),
                        ops: vec![update_file::UpdateOp::Replace {
                            pattern: r"iii\.registerFunction\(.*".into(),
                            replacement:
                                "iii.registerFunction({ body: { message: `Hello, ${name}!` } });"
                                    .into(),
                            ignore_case: false,
                            dot_matches_newline: true,
                            expect_matches: Some(1),
                        }],
                    }],
                },
            )
            .await
            .expect("update-file batches never fail top-level for per-entry errors");
        let wire = out.results[0]
            .error
            .as_ref()
            .expect("entry must carry an error");
        put(
            "C210_replace_undefined_capture_ref",
            (wire.code.clone(), wire.message.clone()),
            &jail,
        );
    }

    // --- C211: move — missing source ------------------------------------
    {
        let out = move_file::handle(
            jail.resolver.clone(),
            move_file::MoveFileInput {
                files: vec![move_file::MoveFileSpec {
                    from: "no-such-move-src.txt".into(),
                    to: "no-such-dst.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
            },
        )
        .await
        .expect("move batches never fail top-level");
        let wire = out.results[0]
            .error
            .as_ref()
            .expect("entry must carry an error");
        put(
            "C211_move_missing_src",
            (wire.code.clone(), wire.message.clone()),
            &jail,
        );
    }

    // --- compare against the committed golden ---------------------------
    let mut pretty = serde_json::to_string_pretty(&cases).expect("cases serialize");
    pretty.push('\n');
    support::assert_golden("errors.json", &pretty);
}
