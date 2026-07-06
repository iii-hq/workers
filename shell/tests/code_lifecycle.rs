//! End-to-end lifecycle for the folded `coder::*` code surface, driven
//! through the REAL per-verb handlers (the same async fns `register_all`
//! wires to the engine) over a single shared multi-root jail.
//!
//! The standalone `coder` worker had a subprocess integration test that
//! booted the `iii` engine and a `coder` binary, then drove them over
//! `iii-sdk`; it self-skipped whenever `iii` was absent from `PATH`. After
//! the fold there is no `coder` binary, and the engine path is already
//! covered by the TypeScript e2e harness under `tests/e2e/`. This test
//! keeps the SAME behavioral sequence — create -> read -> update -> read
//! back -> list -> tree -> search -> non-accessible block -> delete — but
//! calls the handlers directly so it is deterministic and always runs.
//!
//! Each call deserializes the exact wire payload an agent would send
//! (`serde_json::from_value` into the typed input) and asserts on the
//! serialized wire output, so the shapes pinned here are the ones the
//! engine actually returns.

use std::sync::Arc;

use serde_json::{json, Value};

use shell::code::config::CoderConfig;
use shell::code::functions::{
    create_file, delete_file, list_folder, read_file, search, tree, update_file,
};
use shell::code::path::PathResolver;

/// A jailed code surface rooted at a fresh tempdir, with `**/.env` marked
/// non-accessible (the canonical "visible but locked" secret pattern).
struct Surface {
    _tmp: tempfile::TempDir,
    /// Canonical jail root — response paths are canonical-absolute, so
    /// expectations anchor here.
    root: std::path::PathBuf,
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
}

impl Surface {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).expect("resolver"));
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        Self {
            _tmp: tmp,
            root,
            resolver,
            cfg,
        }
    }
}

fn input<T: serde::de::DeserializeOwned>(payload: Value) -> T {
    serde_json::from_value(payload).expect("wire payload must deserialize into the typed input")
}

fn wire<T: serde::Serialize>(out: T) -> Value {
    serde_json::to_value(out).expect("output serializes to wire JSON")
}

#[tokio::test]
async fn full_lifecycle_via_handlers() {
    let s = Surface::new();

    // 1. Create a file.
    let create = wire(
        create_file::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({
                "files": [{
                    "path": "hello.txt",
                    "content": "hello world\nsecond line\n",
                    "mode": "0644",
                    "parents": true,
                    "overwrite": false
                }]
            })),
        )
        .await
        .expect("create-file"),
    );
    let create_results = create["results"].as_array().expect("create results array");
    assert_eq!(create_results.len(), 1);
    assert_eq!(
        create_results[0]["success"], true,
        "create failed: {:?}",
        create_results[0]["error"]
    );

    // 2. Read it back.
    let read = wire(
        read_file::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "path": "hello.txt" })),
        )
        .await
        .expect("read-file"),
    );
    assert_eq!(read["content"], "hello world\nsecond line\n");
    assert_eq!(read["size"], 24);

    // 3. Update — replace line 2.
    let updated = wire(
        update_file::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({
                "files": [{
                    "path": "hello.txt",
                    "ops": [
                        { "op": "update_lines", "from_line": 2, "to_line": 2, "content": "REPLACED" }
                    ]
                }]
            })),
        )
        .await
        .expect("update-file"),
    );
    assert_eq!(updated["results"][0]["success"], true);

    let after = wire(
        read_file::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "path": "hello.txt" })),
        )
        .await
        .expect("read after update"),
    );
    assert_eq!(after["content"], "hello world\nREPLACED\n");

    // 4. list-folder shows the file.
    let list = wire(
        list_folder::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "path": ".", "page": 1 })),
        )
        .await
        .expect("list-folder"),
    );
    let entries = list["entries"].as_array().expect("entries array");
    assert!(entries.iter().any(|e| e["name"] == "hello.txt"));

    // 5. tree returns the file as a child of root.
    let tree = wire(
        tree::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "path": "." })),
        )
        .await
        .expect("tree"),
    );
    let kids = tree["root"]["children"].as_array().expect("root children");
    assert!(kids.iter().any(|k| k["name"] == "hello.txt"));

    // 6. search finds the content. Response paths are canonical-absolute
    //    (decision D2-eng); the resolver canonicalized its root, so anchor
    //    the expectation the same way.
    let found = wire(
        search::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "query": "REPLACED", "regex": false })),
        )
        .await
        .expect("search"),
    );
    let content_matches = found["content_matches"]
        .as_array()
        .expect("content matches");
    let expected_match_path = s.root.join("hello.txt").display().to_string();
    assert!(content_matches
        .iter()
        .any(|m| m["path"] == expected_match_path.as_str() && m["line"] == 2));

    // 7. Non-accessible glob blocks reads even though list-folder shows it.
    std::fs::write(s.root.join(".env"), "API_KEY=secret\n").unwrap();
    let list_with_env = wire(
        list_folder::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "path": ".", "page": 1 })),
        )
        .await
        .expect("list-folder after .env"),
    );
    let env_entry = list_with_env["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == ".env")
        .expect(".env should be listed");
    assert_eq!(env_entry["non_accessible"], true);

    let read_env = read_file::handle(
        s.resolver.clone(),
        s.cfg.clone(),
        input(json!({ "path": ".env" })),
    )
    .await;
    let err_msg = match read_env {
        Ok(_) => panic!("read of .env must be rejected"),
        Err(e) => e,
    };
    assert!(err_msg.contains("C211"), "got: {err_msg}");

    // 8. delete-file removes the file.
    let del = wire(
        delete_file::handle(
            s.resolver.clone(),
            s.cfg.clone(),
            input(json!({ "paths": ["hello.txt"] })),
        )
        .await
        .expect("delete-file"),
    );
    assert_eq!(del["results"][0]["success"], true);
    assert_eq!(del["results"][0]["removed"], true);
}
