// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! No VM or subprocess: every compatibility case must fail BEFORE such work.
use super::*;
use std::os::unix::fs::PermissionsExt;
fn private(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
struct Setup {
    _root: tempfile::TempDir,
    vms: WorkerVms,
    vm: PathBuf,
    custody: PathBuf,
    source: PathBuf,
}
fn setup(compat: bool) -> Setup {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base = root.path().join("workers");
    private(&base);
    let vm = base.join(vm_id("key"));
    private(&vm);
    private(&vm.join("root"));
    std::fs::write(vm.join("root/upper.ext4"), b"OLD-do-not-archive").unwrap();
    std::fs::write(vm.join("state.json"), b"{\"instance\":7}").unwrap();
    let source = root.path().join("source");
    private(&source);
    std::fs::write(source.join("iii.worker.yaml"), b"not-valid-yaml: [").unwrap();
    let custody = crate::worker_admission::custody(&base, &vm_id("key")).unwrap();
    if compat {
        private(custody.parent().unwrap());
        private(&custody);
        std::fs::write(custody.join("active.json"), b"{corrupt").unwrap();
    }
    let vms = WorkerVms::new(base, Some("token".into()), vec![root.path().to_path_buf()]);
    Setup {
        _root: root,
        vms,
        vm,
        custody,
        source,
    }
}
fn source(s: &Setup) -> WorkerSource {
    WorkerSource::Local {
        host_path: s.source.display().to_string(),
        run_override: None,
    }
}
fn start(s: &Setup) -> WorkerStartRequest {
    WorkerStartRequest {
        auth: "token".into(),
        state_key: "key".into(),
        worker_name: "worker".into(),
        source: source(s),
        env: Default::default(),
        engine_url: "ws://127.0.0.1:1/ws".into(),
        build: None,
    }
}
fn unchanged(s: &Setup) {
    assert_eq!(
        std::fs::read(s.vm.join("root/upper.ext4")).unwrap(),
        b"OLD-do-not-archive"
    );
    assert_eq!(
        std::fs::read(s.vm.join("state.json")).unwrap(),
        b"{\"instance\":7}"
    );
    assert!(!s.vm.join("build-dir").exists());
    assert!(!s.vm.join("build-mode").exists());
    assert!(!s.vm.join("upper.ext4").exists());
    assert_eq!(s.vms.starts.load(Ordering::SeqCst), 0);
    assert!(s.vms.slots().is_empty());
    assert!(s.vms.builds().is_empty());
}
#[tokio::test]
async fn start_refuses_compat_before_instance_claim_or_archive() {
    let s = setup(true);
    let error = s.vms.start(start(&s)).await.unwrap_err();
    assert!(error.to_string().contains("compatibility custody"));
    unchanged(&s);
}
#[tokio::test]
async fn build_start_refuses_before_even_reading_or_claiming_build_directory() {
    let s = setup(true);
    let mut req = start(&s);
    req.build = Some(WorkerBuildRef {
        build_id: "not-real".into(),
        build_dir: s._root.path().join("uncreated-build").display().to_string(),
    });
    assert!(
        s.vms
            .start(req)
            .await
            .unwrap_err()
            .to_string()
            .contains("compatibility custody")
    );
    unchanged(&s);
    assert!(!s._root.path().join("uncreated-build").exists());
}
#[tokio::test]
async fn prepare_keyed_and_unkeyed_refuse_before_source_or_cache() {
    let s = setup(true);
    for key in [Some("key".into()), None] {
        let error = s
            .vms
            .prepare(WorkerPrepareRequest {
                auth: "token".into(),
                source: source(&s),
                state_key: key,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("compatibility custody"));
    }
    unchanged(&s);
}
#[tokio::test]
async fn worker_build_refuses_before_output_and_channel() {
    let s = setup(true);
    let iii = IIIClient::new("ws://127.0.0.1:1/ws");
    let out = s._root.path().join("output");
    let req = WorkerBuildRequest {
        auth: "token".into(),
        state_key: "key".into(),
        worker_name: "worker".into(),
        source: source(&s),
        output_dir: out.display().to_string(),
        env: Default::default(),
        logs: iii_sdk::channels::StreamChannelRef {
            channel_id: "never-open".into(),
            access_key: "unused".into(),
            direction: iii_sdk::channels::ChannelDirection::Write,
        },
        base_image: None,
    };
    let error = s.vms.build(&iii, req).await.unwrap_err();
    assert!(error.to_string().contains("compatibility custody"));
    assert!(!out.exists());
    unchanged(&s);
}
#[test]
fn destroy_and_explicit_prune_cannot_remove_compatibility_or_originals() {
    let s = setup(true);
    for prune in [false, true] {
        assert!(
            s.vms
                .destroy(WorkerDestroyRequest {
                    auth: "token".into(),
                    state_key: "key".into(),
                    prune
                })
                .is_err()
        );
        unchanged(&s);
        assert!(s.custody.join("active.json").exists());
    }
}
#[tokio::test]
async fn authorization_still_precedes_compatibility_inspection() {
    let s = setup(true);
    let mut req = start(&s);
    req.auth = "foreign".into();
    assert!(matches!(
        s.vms.start(req).await,
        Err(SandboxError::Unauthorized)
    ));
    unchanged(&s);
}
#[test]
fn direct_launch_is_guarded_before_command_is_spawned() {
    let s = setup(true);
    let cmd = Command::new("must-never-spawn-this-fixture");
    assert!(
        s.vms
            .launch("key", cmd, vec![], None)
            .unwrap_err()
            .to_string()
            .contains("compatibility custody")
    );
    unchanged(&s);
}
#[tokio::test]
async fn normal_missing_manifest_keeps_prior_validation_and_no_effects() {
    let s = setup(false);
    let error = s
        .vms
        .prepare(WorkerPrepareRequest {
            auth: "token".into(),
            source: source(&s),
            state_key: Some("key".into()),
        })
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("compatibility custody"));
    unchanged(&s);
}
#[test]
fn normal_destroy_retains_data_and_explicit_prune_still_removes_it() {
    let s = setup(false);
    let retained = s
        .vms
        .destroy(WorkerDestroyRequest {
            auth: "token".into(),
            state_key: "key".into(),
            prune: false,
        })
        .unwrap();
    assert!(!retained.kept.is_empty());
    assert!(std::fs::read_dir(&s.vm).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("upper.ext4.pre-build")
    }));
    let removed = s
        .vms
        .destroy(WorkerDestroyRequest {
            auth: "token".into(),
            state_key: "key".into(),
            prune: true,
        })
        .unwrap();
    assert!(removed.destroyed);
    assert!(!s.vm.exists());
}
#[tokio::test]
async fn local_path_bundle_binary_and_build_helpers_refuse_before_runtime_work() {
    use crate::cli::local_worker::{BootImage, VmOverride};
    let s = setup(true);
    let over = || VmOverride {
        state_dir: s.vm.join("root"),
        engine_url: "ws://127.0.0.1:1/ws",
        extra_env: Default::default(),
        image: None,
    };
    let local = crate::cli::local_worker::local_vm_command("w", &s.source, None, over())
        .await
        .err()
        .unwrap();
    assert!(local.contains("compatibility custody"));
    let bundle = crate::cli::local_worker::bundle_vm_command("pkg", "w", &s.source, over())
        .await
        .err()
        .unwrap();
    assert!(bundle.contains("compatibility custody"));
    let mut binary = over();
    binary.image = Some(BootImage {
        image: s._root.path().join("image"),
        upper: s.vm.join("upper.ext4"),
    });
    assert!(
        crate::cli::local_worker::binary_vm_command("pkg", "bin/w", "w", &s.source, binary)
            .await
            .err()
            .unwrap()
            .contains("compatibility custody")
    );
    assert!(
        crate::cli::local_worker::image_build_command(
            &s.source,
            None,
            None,
            None,
            None,
            &s.vm.join("build-1"),
            Default::default()
        )
        .await
        .err()
        .unwrap()
        .contains("compatibility custody")
    );
    unchanged(&s);
}
