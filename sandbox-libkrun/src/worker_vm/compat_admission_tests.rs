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

// A is normal; B's original root is outside custody itself. Only the bound
// sibling namespace establishes refusal. This fixture grants no authority.
fn enrolled_other_root(s: &Setup) -> PathBuf {
    let id = vm_id("other-key");
    let vm = s.vms.base.join(&id);
    private(&vm);
    let root = vm.join("root");
    private(&root);
    std::fs::write(root.join("original"), b"B-original").unwrap();
    let custody = crate::worker_admission::custody(&s.vms.base, &id).unwrap();
    private(custody.parent().unwrap());
    private(&custody);
    std::fs::write(custody.join("active.json"), b"{corrupt").unwrap();
    root
}

fn local_source(path: &Path) -> WorkerSource {
    WorkerSource::Local {
        host_path: path.display().to_string(),
        run_override: None,
    }
}

#[tokio::test]
async fn normal_a_refuses_b_original_source_before_instance_or_source_fallback() {
    let s = setup(false);
    let original = enrolled_other_root(&s);
    let mut req = start(&s);
    req.auth = crate::wire::scoped_auth_token("token", "key");
    req.source = local_source(&original);
    let error = s.vms.start(req).await.unwrap_err();
    assert!(
        error.to_string().contains("compatibility custody"),
        "{error}"
    );
    unchanged(&s);
    assert_eq!(
        std::fs::read(original.join("original")).unwrap(),
        b"B-original"
    );
}

#[tokio::test]
async fn keyed_prepare_refuses_b_original_before_manifest_defaults_or_download() {
    let s = setup(false);
    let original = enrolled_other_root(&s);
    let error = s
        .vms
        .prepare(WorkerPrepareRequest {
            auth: "token".into(),
            state_key: Some("key".into()),
            source: local_source(&original),
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("compatibility custody"),
        "{error}"
    );
    unchanged(&s);
    assert_eq!(std::fs::read_dir(&original).unwrap().count(), 1);
}

#[tokio::test]
async fn normal_source_with_b_build_dir_refuses_before_claim_instance_or_archive() {
    let s = setup(false);
    let original = enrolled_other_root(&s);
    let mut req = start(&s);
    req.auth = crate::wire::scoped_auth_token("token", "key");
    req.build = Some(WorkerBuildRef {
        build_dir: original.display().to_string(),
        build_id: "never-read".into(),
    });
    let error = s.vms.start(req).await.unwrap_err();
    assert!(
        error.to_string().contains("compatibility custody"),
        "{error}"
    );
    unchanged(&s);
    assert_eq!(std::fs::read_dir(&original).unwrap().count(), 1);
}

#[tokio::test]
async fn build_refuses_foreign_source_or_absent_foreign_output_before_claim_or_channel() {
    for retained_source in [false, true] {
        let s = setup(false);
        let original = enrolled_other_root(&s);
        let output = if retained_source {
            s._root.path().join("new-output")
        } else {
            original.join("absent/output")
        };
        let iii = IIIClient::new("ws://127.0.0.1:1/ws");
        let req = WorkerBuildRequest {
            auth: crate::wire::scoped_auth_token("token", "key"),
            state_key: "key".into(),
            worker_name: "worker".into(),
            source: local_source(if retained_source {
                &original
            } else {
                &s.source
            }),
            output_dir: output.display().to_string(),
            env: Default::default(),
            logs: iii_sdk::channels::StreamChannelRef {
                channel_id: "never-open".into(),
                access_key: "unused".into(),
                direction: iii_sdk::channels::ChannelDirection::Write,
            },
            base_image: None,
        };
        let error = s.vms.build(&iii, req).await.unwrap_err();
        assert!(
            error.to_string().contains("compatibility custody"),
            "{error}"
        );
        assert!(!output.exists());
        assert!(!original.join("absent").exists());
        unchanged(&s);
    }
}

#[tokio::test]
async fn normal_root_and_source_refuse_foreign_image_or_upper_at_outer_builders() {
    let s = setup(false);
    let original = enrolled_other_root(&s);
    for retained_image in [false, true] {
        let over = || VmOverride {
            state_dir: s.vm.join("root"),
            engine_url: "ws://127.0.0.1:1/ws",
            extra_env: Default::default(),
            image: Some(BootImage {
                image: if retained_image {
                    original.join("image.erofs")
                } else {
                    s.source.join("image.erofs")
                },
                upper: if retained_image {
                    s.vm.join("upper.ext4")
                } else {
                    original.join("upper.ext4")
                },
            }),
        };
        // An invalid manifest/executable is a second, effect-free sentinel if
        // the early guard regresses; these cases never need real firmware.
        let local = local_vm_command("w", &s.source, None, over())
            .await
            .err()
            .unwrap();
        assert!(local.contains("compatibility custody"), "{local}");
        let bundle = bundle_vm_command("pkg", "w", &s.source, over())
            .await
            .err()
            .unwrap();
        assert!(bundle.contains("compatibility custody"), "{bundle}");
        let binary = binary_vm_command("pkg", "missing", "w", &s.source, over())
            .await
            .err()
            .unwrap();
        assert!(binary.contains("compatibility custody"), "{binary}");
        unchanged(&s);
    }
}

#[tokio::test]
async fn foreign_inputs_do_not_precede_authentication() {
    let s = setup(false);
    let original = enrolled_other_root(&s);
    let mut req = start(&s);
    req.auth = "foreign".into();
    req.source = local_source(&original);
    assert!(matches!(
        s.vms.start(req).await,
        Err(SandboxError::Unauthorized)
    ));
    let error = s
        .vms
        .prepare(WorkerPrepareRequest {
            auth: "foreign".into(),
            state_key: Some("key".into()),
            source: local_source(&original),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SandboxError::Unauthorized));
    unchanged(&s);
}

#[tokio::test]
async fn source_symlink_is_not_hidden_by_host_root_canonicalization() {
    let s = setup(false);
    let link = s._root.path().join("source-link");
    std::os::unix::fs::symlink(&s.source, &link).unwrap();
    let mut req = start(&s);
    req.source = local_source(&link);
    assert!(matches!(
        s.vms.start(req).await,
        Err(SandboxError::BootFailed(_))
    ));
    unchanged(&s);
}

#[tokio::test]
async fn image_build_refuses_foreign_source_before_default_image_or_firmware() {
    let s = setup(false);
    let original = enrolled_other_root(&s);
    let scratch = s.vm.join("build-not-created");
    let error = image_build_command(
        &original,
        None,
        None,
        None,
        None,
        &scratch,
        Default::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("compatibility custody"), "{error}");
    assert!(!scratch.exists());
    unchanged(&s);
}

fn valid_project(s: &Setup) {
    std::fs::write(
        s.source.join("iii.worker.yaml"),
        "name: worker\nruntime:\n  base_image: docker.io/library/alpine:3.20\nscripts:\n  start: echo worker\n",
    ).unwrap();
}

fn refused_cache(s: &Setup, sidecar: bool) -> PathBuf {
    let cache = s._root.path().join("cache");
    let target = s._root.path().join("cache-target");
    private(&target);
    if sidecar {
        private(&cache);
        let slug = oci::rootfs_slug_for_image("docker.io/library/alpine:3.20");
        std::os::unix::fs::symlink(&target, cache.join(format!("{slug}.digest"))).unwrap();
    } else {
        std::os::unix::fs::symlink(&target, &cache).unwrap();
    }
    cache
}

#[tokio::test]
async fn start_cache_refusal_leaves_reservation_counters_and_state_untouched() {
    for sidecar in [false, true] {
        let s = setup(false);
        valid_project(&s);
        let cache = refused_cache(&s, sidecar);
        let checked = std::cell::Cell::new(false);
        let result = s
            .vms
            .start_with_cache_admission(start(&s), |project, admission| {
                checked.set(true);
                // Same admission implementation, with an own-temp cache root only.
                project.admit_cache_at(admission, &cache)
            })
            .await;
        assert!(checked.get(), "must reach the actual cache admission");
        assert!(matches!(result, Err(SandboxError::BootFailed(_))));
        unchanged(&s);
        assert_eq!(s.vms.build_seq.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read_dir(s._root.path().join("cache-target"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn build_cache_refusal_precedes_claim_output_sweeps_and_scratch_allocation() {
    for sidecar in [false, true] {
        for missing_output in [false, true] {
            let s = setup(false);
            valid_project(&s);
            let cache = refused_cache(&s, sidecar);
            // Canonical VM storage permits absent tails under the fixed owner
            // domain. Generic absent output parents are a separate refusal gate.
            let out = s.vm.join("build-output");
            if !missing_output {
                private(&out);
                std::fs::write(out.join(".image.erofs.1-0.partial"), b"keep-partial").unwrap();
            }
            let stale = s.vm.join("build-1-0");
            private(&stale);
            std::fs::write(stale.join("sentinel"), b"keep-scratch").unwrap();
            let checked = std::cell::Cell::new(false);
            let iii = IIIClient::new("ws://127.0.0.1:1/ws");
            let req = WorkerBuildRequest {
                auth: crate::wire::scoped_auth_token("token", "key"),
                state_key: "key".into(),
                worker_name: "worker".into(),
                source: source(&s),
                output_dir: out.display().to_string(),
                env: Default::default(),
                base_image: None,
                logs: iii_sdk::channels::StreamChannelRef {
                    channel_id: "never-open".into(),
                    access_key: "unused".into(),
                    direction: iii_sdk::channels::ChannelDirection::Write,
                },
            };
            let result = s
                .vms
                .build_with_cache_admission(&iii, req, |project, admission| {
                    checked.set(true);
                    project.admit_cache_at(admission, &cache)
                })
                .await;
            assert!(
                checked.get(),
                "must reach cache admission before overlay/claim"
            );
            assert!(matches!(result, Err(SandboxError::BuildFailed(_))));
            unchanged(&s);
            assert_eq!(s.vms.build_seq.load(Ordering::SeqCst), 0);
            assert_eq!(
                std::fs::read(stale.join("sentinel")).unwrap(),
                b"keep-scratch"
            );
            assert!(
                !s.vm
                    .join(format!("build-{}-0", std::process::id()))
                    .exists()
            );
            if missing_output {
                assert!(!out.exists());
            } else {
                assert_eq!(
                    std::fs::read(out.join(".image.erofs.1-0.partial")).unwrap(),
                    b"keep-partial"
                );
            }
        }
    }
}

#[tokio::test]
async fn symlink_image_id_refuses_external_start_and_internal_claim_before_effects() {
    let s = setup(false);
    valid_project(&s);
    let dir = s._root.path().join("build-image");
    private(&dir);
    std::fs::write(dir.join(IMAGE_FILE), b"normal-image").unwrap();
    let target = s._root.path().join("id-target");
    std::fs::write(&target, b"sha256:fixture\n").unwrap();
    std::os::unix::fs::symlink(&target, dir.join(IMAGE_ID_FILE)).unwrap();
    let build = WorkerBuildRef {
        build_dir: dir.display().to_string(),
        build_id: "sha256:fixture".into(),
    };
    let auth = crate::wire::scoped_auth_token("token", "key");
    let mut req = start(&s);
    req.auth = auth.clone();
    req.build = Some(build.clone());
    assert!(matches!(
        s.vms.start(req).await,
        Err(SandboxError::BootFailed(_))
    ));
    unchanged(&s);
    assert!(matches!(
        s.vms.admit_build(&auth, "key", &build),
        Err(SandboxError::BootFailed(_))
    ));
    unchanged(&s);
    assert_eq!(s.vms.build_seq.load(Ordering::SeqCst), 0);
    assert!(
        read_image_id(&dir).is_none(),
        "O_NOFOLLOW must remain in place"
    );
    assert_eq!(
        std::fs::read(dir.join(IMAGE_FILE)).unwrap(),
        b"normal-image"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"sha256:fixture\n");
    assert_eq!(std::fs::read_link(dir.join(IMAGE_ID_FILE)).unwrap(), target);
}
