//! End-to-end pipeline against a real React workspace: discover, build with
//! Vite, hash inputs, write a line, change a shared file, rebuild, compare.
//! Needs Node and an installed project, so it only runs when
//! `III_STORIES_FIXTURE` (a workspace with story files) and
//! `III_STORIES_COMPILER_DIR` (a compiler folder with node_modules) are set.

use std::path::{Path, PathBuf};

use iii_stories::builder::component_from;
use iii_stories::compiler::Compiler;
use iii_stories::model::{ChangeKind, Index, LineInfo, LineKind, compare, summarize};
use iii_stories::store::{Manifest, Store, WORKTREE_KEY};

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

async fn build_index(
    compiler: &Compiler,
    store: &Store,
    root: &Path,
    key: &str,
) -> (Index, Manifest) {
    let projects = compiler.discover(root, &[], &[]).await.expect("discover");
    assert!(!projects.is_empty(), "fixture has no story files");
    let mut manifest = Manifest::new();
    let mut components = Vec::new();
    for project in &projects {
        let dir = if project.path == "." {
            root.to_path_buf()
        } else {
            root.join(&project.path)
        };
        let out = store
            .tmp_dir("ws", key)
            .join(project.path.replace('/', "-"));
        let output = compiler
            .build(&dir, &out, &project.files, None, None, &[], &[])
            .await
            .expect("build");
        assert!(output.ok, "{:?}", output.error);
        for file in &output.files {
            assert!(file.error.is_none(), "{}: {:?}", file.file, file.error);
            components.push(component_from(store, "ws", root, project, file));
        }
        store
            .import_dist("ws", &out, &project.path, &mut manifest)
            .expect("import");
    }
    let index = Index {
        workspace: "ws".into(),
        line: LineInfo {
            key: key.into(),
            kind: LineKind::Worktree,
            label: key.into(),
            sha: None,
            dirty: None,
            built_at: "t".into(),
        },
        projects: vec![],
        components,
        warnings: vec![],
    };
    store
        .write_line("ws", &index, &manifest)
        .expect("write line");
    (index, manifest)
}

#[tokio::test]
#[ignore = "needs III_STORIES_FIXTURE and III_STORIES_COMPILER_DIR"]
async fn builds_indexes_and_compares_a_real_workspace() {
    let (Some(fixture), Some(compiler_dir)) = (
        env_path("III_STORIES_FIXTURE"),
        env_path("III_STORIES_COMPILER_DIR"),
    ) else {
        eprintln!("skipped: set III_STORIES_FIXTURE and III_STORIES_COMPILER_DIR");
        return;
    };
    let data = tempfile::tempdir().unwrap();
    let store = Store::new(data.path().to_path_buf());
    let compiler = Compiler::ensure(compiler_dir).await.expect("compiler");

    let (before, manifest) = build_index(&compiler, &store, &fixture, WORKTREE_KEY).await;
    assert!(
        before.components.len() >= 3,
        "expected the fixture's three story files"
    );
    let button = before
        .components
        .iter()
        .find(|c| c.id == "ui-button")
        .expect("ui-button");
    assert_eq!(button.project, "client-app");
    assert!(button.states.iter().any(|s| s.id == "ui-button--primary"));
    assert!(
        button
            .inputs
            .iter()
            .any(|i| i.path == "client-app/src/Button.tsx" && i.hop == 1)
    );
    assert!(
        button
            .inputs
            .iter()
            .any(|i| i.path == "client-app/src/lib/cn.ts" && i.hop == 2)
    );
    let html = button.html.clone().expect("html");
    assert!(
        manifest.contains_key(&html),
        "served html {html} missing from manifest"
    );
    let (bytes, ty) = store.serve("ws", WORKTREE_KEY, &html).expect("serve html");
    assert!(ty.starts_with("text/html") && bytes.starts_with(b"<!doctype html>"));
    let card = before
        .components
        .iter()
        .find(|c| c.id == "ui-card")
        .expect("ui-card");
    assert!(
        card.inputs
            .iter()
            .any(|i| i.path == "packages/ui/src/Badge.tsx"),
        "cross-project input resolved to its real path"
    );
    let status = before
        .components
        .iter()
        .find(|c| c.id == "admin-status")
        .expect("admin-status");
    assert!(
        status
            .inputs
            .iter()
            .any(|i| i.path == "packages/ui/src/Badge.tsx")
    );

    // Touch the shared Badge: Card (client-app) and Status (admin-app) go
    // indirect/direct according to their hop; Button stays put.
    let badge = fixture.join("packages/ui/src/Badge.tsx");
    let original = std::fs::read_to_string(&badge).unwrap();
    std::fs::write(
        &badge,
        format!("{original}\n// touched by the pipeline test\n"),
    )
    .unwrap();
    let result = std::panic::AssertUnwindSafe(async {
        store.rotate_worktree("ws").unwrap();
        build_index(&compiler, &store, &fixture, WORKTREE_KEY)
            .await
            .0
    });
    let after = result.await;
    std::fs::write(&badge, original).unwrap();

    let changes = compare(&before, &after);
    let kind = |id: &str| changes.iter().find(|c| c.id == id).map(|c| c.kind).unwrap();
    assert_eq!(kind("ui-button"), ChangeKind::Unchanged);
    assert_eq!(
        kind("ui-card"),
        ChangeKind::Indirect,
        "Badge is two hops from Card.stories"
    );
    assert_eq!(
        kind("admin-status"),
        ChangeKind::Indirect,
        "Badge is two hops from Status.stories (through the package index)"
    );
    let card_change = changes.iter().find(|c| c.id == "ui-card").unwrap();
    assert_eq!(card_change.files[0].path, "packages/ui/src/Badge.tsx");
    let summary = summarize(&changes);
    assert_eq!((summary.indirect, summary.unchanged), (2, 1));
    assert_eq!(store.list_lines("ws").len(), 2);
}
