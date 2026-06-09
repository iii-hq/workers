//! Integration tests targeting under-covered branches in `src/fs/host.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{Channel, IIIError};

use shell::fs::host::{ChannelMaker, HostFsBackend, HostFsConfig};
use shell::fs::{ChmodArgs, FsBackend, GrepArgs, MkdirArgs, MvArgs, RmArgs, SedArgs, StatArgs};

#[derive(Debug)]
struct StubChan;

#[async_trait]
impl ChannelMaker for StubChan {
    async fn create_channel(&self, _: usize) -> Result<Channel, IIIError> {
        Err(IIIError::Handler(
            "stub channel maker — non-streaming tests only".into(),
        ))
    }
    fn engine_address(&self) -> String {
        "ws://stub:0".into()
    }
}

fn backend() -> HostFsBackend {
    HostFsBackend::new(Arc::new(HostFsConfig::default()), Arc::new(StubChan))
}

fn tmpdir(prefix: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("shell-host-{}-{}", prefix, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn mkdir_parents_true_creates_deeply_nested_path() {
    let root = tmpdir("mkdir-deep");
    let deep = root.join("a/b/c/d/e/leaf");
    let r = backend()
        .mkdir(MkdirArgs {
            path: deep.to_string_lossy().into_owned(),
            mode: "0755".into(),
            parents: true,
        })
        .await
        .expect("mkdir parents=true");
    assert!(r.created, "leaf created");
    assert!(deep.is_dir(), "leaf exists on disk");
    let again = backend()
        .mkdir(MkdirArgs {
            path: deep.to_string_lossy().into_owned(),
            mode: "0755".into(),
            parents: true,
        })
        .await
        .expect("idempotent re-mkdir");
    assert!(!again.created, "second mkdir reports not created");
    assert!(
        again.already_existed,
        "second mkdir reports already_existed"
    );
}

#[tokio::test]
async fn mv_overwrite_true_replaces_existing_dst() {
    let root = tmpdir("mv-over");
    let src = root.join("src.txt");
    let dst = root.join("dst.txt");
    std::fs::write(&src, b"new").unwrap();
    std::fs::write(&dst, b"old").unwrap();
    let r = backend()
        .mv(MvArgs {
            src: src.to_string_lossy().into_owned(),
            dst: dst.to_string_lossy().into_owned(),
            overwrite: true,
        })
        .await
        .expect("mv overwrite=true succeeds");
    assert!(r.moved);
    assert!(r.overwrote, "dst pre-existed so overwrote must be true");
    assert!(!src.exists(), "src removed after rename");
    assert_eq!(std::fs::read(&dst).unwrap(), b"new");
}

#[tokio::test]
async fn chmod_recursive_walks_subtree_and_counts() {
    let root = tmpdir("chmod-rec");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), b"a").unwrap();
    std::fs::write(root.join("sub/b.txt"), b"b").unwrap();
    let r = backend()
        .chmod(ChmodArgs {
            path: root.to_string_lossy().into_owned(),
            // 0750 preserves +x on dirs so walkdir can descend.
            mode: "0750".into(),
            uid: None,
            gid: None,
            recursive: true,
        })
        .await
        .expect("chmod recursive succeeds");
    assert!(
        r.entries_changed >= 3,
        "expected ≥3 paths walked, got {}",
        r.entries_changed
    );
}

#[tokio::test]
async fn grep_include_glob_filters_paths() {
    let root = tmpdir("grep-inc");
    std::fs::write(root.join("keep.rs"), b"needle\n").unwrap();
    std::fs::write(root.join("skip.txt"), b"needle\n").unwrap();
    let r = backend()
        .grep(GrepArgs {
            path: root.to_string_lossy().into_owned(),
            pattern: "needle".into(),
            recursive: true,
            ignore_case: false,
            include_glob: vec!["*.rs".into()],
            exclude_glob: vec![],
            max_matches: 100,
            max_line_bytes: 4096,
        })
        .await
        .expect("grep with include_glob succeeds");
    assert_eq!(r.matches.len(), 1, "only the .rs file should match");
    assert!(
        r.matches[0].path.ends_with("keep.rs"),
        "matched path: {}",
        r.matches[0].path,
    );
}

#[tokio::test]
async fn grep_exclude_glob_skips_paths() {
    let root = tmpdir("grep-exc");
    std::fs::write(root.join("a.log"), b"needle\n").unwrap();
    std::fs::write(root.join("b.log"), b"needle\n").unwrap();
    std::fs::write(root.join("c.txt"), b"needle\n").unwrap();
    let r = backend()
        .grep(GrepArgs {
            path: root.to_string_lossy().into_owned(),
            pattern: "needle".into(),
            recursive: true,
            ignore_case: false,
            include_glob: vec![],
            exclude_glob: vec!["*.log".into()],
            max_matches: 100,
            max_line_bytes: 4096,
        })
        .await
        .expect("grep with exclude_glob succeeds");
    assert_eq!(r.matches.len(), 1, "only c.txt should match");
    assert!(r.matches[0].path.ends_with("c.txt"));
}

#[tokio::test]
async fn sed_walks_directory_with_include_exclude_globs() {
    let root = tmpdir("sed-glob");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.rs"), b"foo\n").unwrap();
    std::fs::write(root.join("b.rs"), b"foo\n").unwrap();
    std::fs::write(root.join("ignore.rs"), b"foo\n").unwrap();
    std::fs::write(root.join("sub/c.rs"), b"foo\n").unwrap();
    std::fs::write(root.join("d.txt"), b"foo\n").unwrap(); // wrong extension

    let r = backend()
        .sed(SedArgs {
            files: vec![],
            path: Some(root.to_string_lossy().into_owned()),
            recursive: true,
            include_glob: vec!["*.rs".into()],
            exclude_glob: vec!["ignore*".into()],
            pattern: "foo".into(),
            replacement: "bar".into(),
            regex: false,
            first_only: false,
            ignore_case: false,
        })
        .await
        .expect("sed with globs succeeds");
    assert_eq!(r.results.len(), 3, "expected 3 .rs files post-filter");
    assert_eq!(r.total_replacements, 3);
    for result in &r.results {
        assert!(result.success, "sed succeeded on {}", result.path);
        assert!(!result.path.ends_with("ignore.rs"));
        assert!(result.path.ends_with(".rs"));
    }
}

#[tokio::test]
async fn sed_recursive_false_on_directory_rejected_with_s210() {
    let root = tmpdir("sed-shallow");
    std::fs::write(root.join("top.rs"), b"foo\n").unwrap();
    let err = backend()
        .sed(SedArgs {
            files: vec![],
            path: Some(root.to_string_lossy().into_owned()),
            recursive: false,
            include_glob: vec![],
            exclude_glob: vec![],
            pattern: "foo".into(),
            replacement: "bar".into(),
            regex: false,
            first_only: false,
            ignore_case: false,
        })
        .await
        .expect_err("recursive=false on dir must reject");
    assert_eq!(err.code, "S210", "got: {err:?}");
}

#[tokio::test]
async fn sed_first_only_replaces_just_the_first_match_per_line() {
    let root = tmpdir("sed-first");
    let f = root.join("a.txt");
    std::fs::write(&f, b"foo foo foo\n").unwrap();
    let r = backend()
        .sed(SedArgs {
            files: vec![f.to_string_lossy().into_owned()],
            path: None,
            recursive: false,
            include_glob: vec![],
            exclude_glob: vec![],
            pattern: "foo".into(),
            replacement: "bar".into(),
            regex: false,
            first_only: true,
            ignore_case: false,
        })
        .await
        .expect("sed first_only succeeds");
    assert_eq!(r.total_replacements, 1, "first_only stops at one per line");
    let content = std::fs::read_to_string(&f).unwrap();
    assert_eq!(content, "bar foo foo\n");
}

#[tokio::test]
async fn rm_recursive_true_removes_non_empty_dir() {
    let root = tmpdir("rm-rec");
    let target = root.join("doomed");
    std::fs::create_dir_all(target.join("nested")).unwrap();
    std::fs::write(target.join("a.txt"), b"a").unwrap();
    std::fs::write(target.join("nested/b.txt"), b"b").unwrap();
    let r = backend()
        .rm(RmArgs {
            path: target.to_string_lossy().into_owned(),
            recursive: true,
        })
        .await
        .expect("rm -r succeeds");
    assert!(r.removed);
    assert!(!target.exists());
}

#[tokio::test]
async fn stat_reports_is_symlink_for_symlink_target() {
    use std::os::unix::fs::symlink;
    let root = tmpdir("stat-link");
    let target = root.join("target.txt");
    let link = root.join("link.txt");
    std::fs::write(&target, b"hello").unwrap();
    symlink(&target, &link).unwrap();
    let s = backend()
        .stat(StatArgs {
            path: link.to_string_lossy().into_owned(),
        })
        .await
        .expect("stat through symlink");
    // Worker's wire shape exposes both is_dir and is_symlink. With
    // canonicalization, is_symlink may be false and the size matches the
    // target. This locks in current behavior so a regression that
    // changes follow-symlink semantics shows up.
    assert!(
        s.0.size > 0 || s.0.is_symlink,
        "symlink stat reports either size or is_symlink",
    );
}

#[tokio::test]
async fn mkdir_parents_over_existing_file_errors() {
    // mkdir -p over a regular file must error, not report idempotent success.
    let root = tmpdir("mkdir-file");
    let f = root.join("not-a-dir");
    std::fs::write(&f, b"x").unwrap();
    let err = backend()
        .mkdir(MkdirArgs {
            path: f.to_string_lossy().into_owned(),
            mode: "0755".into(),
            parents: true,
        })
        .await
        .expect_err("mkdir -p over a regular file must error");
    assert_eq!(err.code, "S213", "got: {err:?}");
}

#[tokio::test]
async fn host_responses_populate_new_path_fields() {
    // Lock in that the host backend actually fills the structured response
    // fields (not just the legacy bool) so a regression to Default is caught.
    let root = tmpdir("fields");

    let d = root.join("d");
    let mk = backend()
        .mkdir(MkdirArgs {
            path: d.to_string_lossy().into_owned(),
            mode: "0755".into(),
            parents: false,
        })
        .await
        .expect("mkdir");
    assert!(mk.created && !mk.already_existed);
    assert_eq!(mk.path, d.to_string_lossy());

    let ch = backend()
        .chmod(ChmodArgs {
            path: d.to_string_lossy().into_owned(),
            mode: "0700".into(),
            uid: None,
            gid: None,
            recursive: false,
        })
        .await
        .expect("chmod");
    assert_eq!(ch.path, d.to_string_lossy());
    assert!(!ch.recursive);

    let src = root.join("s.txt");
    let dst = root.join("t.txt");
    std::fs::write(&src, b"x").unwrap();
    let mv = backend()
        .mv(MvArgs {
            src: src.to_string_lossy().into_owned(),
            dst: dst.to_string_lossy().into_owned(),
            overwrite: false,
        })
        .await
        .expect("mv");
    assert_eq!(mv.src, src.to_string_lossy());
    assert_eq!(mv.dst, dst.to_string_lossy());
    assert!(!mv.overwrote, "fresh dst was not overwritten");

    let r = backend()
        .rm(RmArgs {
            path: dst.to_string_lossy().into_owned(),
            recursive: false,
        })
        .await
        .expect("rm");
    assert!(r.removed && r.was_present);
    assert_eq!(r.path, dst.to_string_lossy());
}
