// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Compatibility refusals never start a VM. H1 lifecycle regressions below
//! author real owned test children (not executed by the source-only task).
use super::*;
use crate::cli::local_worker::{test_binary_command, test_build_command, test_source_command};
use std::os::unix::fs::PermissionsExt;
fn context(s: &Setup) -> &crate::worker_admission::AdmissionContext {
    s.vms.admission_context.as_ref().unwrap()
}
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
    let catalog = root.path().join("catalog");
    private(&catalog);
    let context = crate::worker_admission::testing::provision_context(&catalog).unwrap();
    let vms = WorkerVms::new(base, Some("token".into()), vec![root.path().to_path_buf()])
        .with_admission_context(context);
    Setup {
        _root: root,
        vms,
        vm,
        custody,
        source,
    }
}
const H1_A: &str = "docker.io/library/alpine:3.18";
const H1_B: &str = "docker.io/library/alpine:3.19";

fn h1_project(s: &Setup, reference: &str) -> WorkerProject {
    WorkerProject::resolve(&s.source, None, None, None, Some(reference), true).unwrap()
}
fn h1_cache(s: &Setup) -> PathBuf {
    let cache = s._root.path().join("cache");
    crate::cli::rootfs_cache::prepare_cache_parent(context(s), &cache).unwrap();
    cache
}
async fn h1_prepare(
    s: &Setup,
    reference: &str,
    produce: bool,
) -> Result<crate::cli::rootfs_cache::PreparedCache, String> {
    let mut project = h1_project(s, reference);
    let lease = s.vms.admission(Some("key")).unwrap();
    s.vms
        .prepare_project_at(
            Some("key"),
            &mut project,
            lease,
            &h1_cache(s),
            |request| async move {
                request
                    .resolve_fixture(produce)
                    .await
                    .map_err(|e| format!("{e:#}"))?
                    .ok_or_else(|| "fixture cache miss".to_string())
            },
        )
        .await
}
async fn h1_seed_disjoint(s: &Setup, reference: &str) {
    let owner = context(s)
        .local(&s._root.path().join("seed-consumer"))
        .unwrap();
    let prepared = crate::cli::rootfs_cache::CacheRequest::new(owner, reference, &h1_cache(s))
        .unwrap()
        .resolve_fixture(true)
        .await
        .unwrap()
        .unwrap();
    drop(prepared); // independent seed writer and consumer are genuinely closed
}
async fn h1_runtime(
    s: &Setup,
    lease: crate::worker_admission::Lease,
    reads: Vec<PathBuf>,
    endpoints: Vec<PathBuf>,
    skip_bind: bool,
) -> std::sync::Arc<crate::sandbox_daemon::adapters::RuntimeSession> {
    crate::sandbox_daemon::adapters::runtime_test_session_resources(
        lease,
        &s._root.path().join("catalog"),
        false,
        crate::sandbox_daemon::adapters::RuntimeTestResources {
            reads,
            endpoints,
            skip_bind,
            exit_without_drop: true,
            ..Default::default()
        },
    )
    .await
}
async fn h1_retire(
    s: &Setup,
    prepared: &crate::cli::rootfs_cache::PreparedCache,
) -> crate::worker_admission::WriterRetirement {
    let session = h1_runtime(
        s,
        prepared.lease.clone(),
        vec![prepared.root.clone(), prepared.lower.clone()],
        Vec::new(),
        false,
    )
    .await;
    let (status, retirement) = session.wait().await;
    assert!(status.unwrap().success());
    let receipt = retirement.unwrap();
    session.finish_resources(&receipt).await.unwrap();
    s.vms
        .retired_builds
        .lock()
        .unwrap()
        .insert(vm_id("key"), receipt.clone());
    receipt
}
fn h1_assert_current(s: &Setup, reference: &str) {
    let stored = s
        .vms
        .prepared_caches
        .lock()
        .unwrap()
        .get("key")
        .unwrap()
        .clone();
    assert_eq!(stored.reference, reference);
    assert_eq!(
        stored.root,
        crate::cli::rootfs_cache::cache_candidates(&h1_cache(s), reference)[0]
    );
}

#[tokio::test]
async fn h1_repeat_build_new_warm_or_cold_reference_keeps_original_operation_and_updates_receipt() {
    for warm in [false, true] {
        let s = setup(false);
        valid_project(&s);
        let original = s.vms.admission(Some("key")).unwrap();
        let a = h1_prepare(&s, H1_A, true).await.unwrap();
        let a_receipt = h1_retire(&s, &a).await;
        if warm {
            h1_seed_disjoint(&s, H1_B).await;
        }
        let scratch = s.vm.join(format!("{BUILD_SCRATCH_PREFIX}current"));
        // Repeat the production declaration set before H1 issuance. These are
        // exact original declarations, not a newly blessed mutable subtree.
        crate::worker_admission::with_paths(
            original.clone(),
            &[
                crate::worker_admission::PathRequest {
                    path: &scratch,
                    role: crate::worker_admission::PathRole::MutableTree,
                },
                crate::worker_admission::PathRequest {
                    path: &scratch.join("root"),
                    role: crate::worker_admission::PathRole::MutableTree,
                },
                crate::worker_admission::PathRequest {
                    path: &scratch.join("root/runtime"),
                    role: crate::worker_admission::PathRole::MutableTree,
                },
                crate::worker_admission::PathRequest {
                    path: &scratch.join(crate::cli::upper::UPPER_NAME),
                    role: crate::worker_admission::PathRole::MutableFile,
                },
            ],
        )
        .unwrap();
        let b = h1_prepare(&s, H1_B, !warm).await.unwrap();
        let mut reset = original
            .begin(crate::worker_admission::MutationPlan::Reclone {
                root: scratch.clone(),
            })
            .unwrap();
        let stage = reset.stage_tree(&scratch).unwrap();
        reset.publish_tree(stage).unwrap();
        reset.commit().unwrap();
        assert_ne!(a.root, b.root);
        h1_assert_current(&s, H1_B);
        // The original pre-handoff Arc sees B: a replacement lifecycle would
        // neither possess this declaration nor validate the real receipt below.
        original.use_path(&b.root).unwrap().revalidate().unwrap();
        original.use_path(&a.root).unwrap().revalidate().unwrap();
        let competitor = context(&s)
            .local(&s._root.path().join("competitor"))
            .unwrap();
        assert!(
            competitor
                .prepare_cache(&[crate::worker_admission::CacheEntry { root: &a.root }])
                .is_err()
        );
        assert!(
            competitor
                .prepare_cache(&[crate::worker_admission::CacheEntry { root: &b.root }])
                .is_err()
        );
        let b_receipt = h1_retire(&s, &b).await;
        let next = h1_cache(&s).join("next-reference");
        assert!(
            a_receipt
                .probe_cache(
                    &original,
                    &[crate::worker_admission::CacheEntry { root: &next }]
                )
                .is_err()
        );
        b_receipt
            .probe_cache(
                &original,
                &[crate::worker_admission::CacheEntry { root: &next }],
            )
            .unwrap()
            .cancel()
            .unwrap();
        // Reuse the SAME reference without any new request factory or producer.
        let mut same = h1_project(&s, H1_B);
        s.vms
            .prepare_project_at(Some("key"), &mut same, original, &h1_cache(&s), |_| async {
                panic!("same-reference reuse must not probe or prepare")
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn h1_failure_preserves_prepared_a_for_missing_foreign_used_and_stale_receipts() {
    for failure in [
        "missing",
        "foreign",
        "used",
        "stale",
        "host-hold",
        "no-pull",
        "producer",
        "changed-after-issuance",
    ] {
        let s = setup(false);
        valid_project(&s);
        let a = h1_prepare(&s, H1_A, true).await.unwrap();
        let receipt = h1_retire(&s, &a).await;
        match failure {
            "missing" => {
                s.vms.retired_builds.lock().unwrap().remove(&vm_id("key"));
            }
            "foreign" => {
                let other = context(&s)
                    .local(&s._root.path().join("foreign-owner"))
                    .unwrap();
                let session = h1_runtime(&s, other, Vec::new(), Vec::new(), false).await;
                let foreign = session.wait().await.1.unwrap();
                session.finish_resources(&foreign).await.unwrap();
                s.vms
                    .retired_builds
                    .lock()
                    .unwrap()
                    .insert(vm_id("key"), foreign);
            }
            "used" => {
                // Consume only an absent, originally declared scratch target;
                // keep the rest of the operation valid so this tests the used
                // receipt, not an unresolved dropped mutation.
                let target = s.vm.join(format!("{BUILD_SCRATCH_PREFIX}current"));
                let mut removal = receipt.begin_remove(&a.lease, &target, &[&target]).unwrap();
                removal.remove_path(&target).unwrap();
                removal.commit().unwrap();
            }
            "stale" => {
                let later = a.lease.prepare_runtime(&[]).unwrap();
                // Cancellation records the later runtime epoch, but this whole
                // lifecycle has already run A: it must NOT receive NeverSpawned
                // cleanup authority. Both facts are required by this negative.
                assert!(later.cancel_before_spawn().is_err());
                let receipt_probe = h1_cache(&s).join("stale-receipt-probe");
                assert!(
                    receipt
                        .probe_cache(
                            &a.lease,
                            &[crate::worker_admission::CacheEntry {
                                root: &receipt_probe,
                            }],
                        )
                        .is_err(),
                    "a later cancelled runtime must invalidate A's old receipt"
                );
            }
            _ => {}
        }
        let read = if failure == "host-hold" {
            Some(a.lease.use_path(&a.root).unwrap())
        } else {
            None
        };
        let before = std::fs::read(&a.lower).unwrap();
        let result = if matches!(failure, "no-pull" | "producer" | "changed-after-issuance") {
            let mut project = h1_project(&s, H1_B);
            s.vms
                .prepare_project_at(
                    Some("key"),
                    &mut project,
                    a.lease.clone(),
                    &h1_cache(&s),
                    |request| {
                        if failure == "changed-after-issuance" {
                            // A completed control-file publication revokes an
                            // issued cache binding; the resolver cannot refresh.
                            write_control_admitted(
                                &a.lease,
                                &s.vm,
                                CONSOLE_LOG,
                                b"new host journal",
                            )
                            .unwrap();
                        }
                        async move {
                            let result = if failure == "producer" {
                                request.resolve_fixture_failure().await
                            } else {
                                request.resolve_fixture(false).await
                            };
                            result
                                .map_err(|e| format!("{e:#}"))?
                                .ok_or_else(|| "deliberate no-pull miss".to_string())
                        }
                    },
                )
                .await
        } else {
            h1_prepare(&s, H1_B, true).await
        };
        assert!(result.is_err(), "{failure}");
        assert_eq!(std::fs::read(&a.lower).unwrap(), before);
        assert_eq!(s.vms.starts.load(Ordering::SeqCst), 0);
        assert_eq!(s.vms.build_seq.load(Ordering::SeqCst), 0);
        assert!(!s.vm.join(BUILD_DIR_RECORD).exists());
        assert!(s.vms.slots().is_empty());
        h1_assert_current(&s, H1_A);
        assert!(!crate::cli::rootfs_cache::cache_candidates(&h1_cache(&s), H1_B)[0].exists());
        drop(read);
    }
}

#[tokio::test]
async fn h1_waited_process_cannot_bypass_host_drain_or_endpoint_failure() {
    for endpoint_failure in [false, true] {
        let s = setup(false);
        valid_project(&s);
        let control = s._root.path().join("control");
        private(&control);
        let socket = control.join("shell.sock");
        let lease = s.vms.admission(Some("key")).unwrap();
        let lease = crate::worker_admission::with_paths(
            lease,
            &[crate::worker_admission::PathRequest {
                path: &socket,
                role: crate::worker_admission::PathRole::Endpoint,
            }],
        )
        .unwrap();
        let a = h1_prepare(&s, H1_A, true).await.unwrap();
        let session = h1_runtime(
            &s,
            lease.clone(),
            vec![a.root.clone()],
            vec![socket],
            endpoint_failure,
        )
        .await;
        let (status, retirement) = session.wait().await;
        assert!(status.unwrap().success());
        let receipt = retirement.unwrap();
        if !endpoint_failure {
            session
                .add_test_host_writer(tokio::spawn(async {
                    Err(io::Error::other("drain failure"))
                }))
                .await;
        }
        assert!(session.finish_resources(&receipt).await.is_err());
        // Mirrors the production waiter: never populate a successful caller
        // receipt after host drain/G failure, despite the OS/core completion.
        assert!(
            !s.vms
                .retired_builds
                .lock()
                .unwrap()
                .contains_key(&vm_id("key"))
        );
        assert!(h1_prepare(&s, H1_B, true).await.is_err());
        h1_assert_current(&s, H1_A);
    }
}

#[tokio::test]
async fn h1_vm_receipt_is_required_after_pumps_and_failed_receipt_does_not_replace_a() {
    let s = setup(false);
    valid_project(&s);
    let a = h1_prepare(&s, H1_A, true).await.unwrap();
    // This existing real-child fixture stores a receipt in Vm::retirement only
    // after actual completion and G settlement, as the production waiter does.
    write_control_admitted(&a.lease, &s.vm, CONSOLE_LOG, b"").unwrap();
    install_retired_fixture(&s).await;
    assert!(s.vms.retired_builds.lock().unwrap().is_empty());
    let b = h1_prepare(&s, H1_B, true).await.unwrap();
    assert_ne!(a.root, b.root);
    let slot = s.vms.slots().get(&vm_id("key")).cloned().unwrap();
    let Slot::Booted(vm) = slot else {
        panic!("owned vm slot expected")
    };
    *vm.retirement.lock().unwrap() = Some(Err("host pump failed".into()));
    let next = "docker.io/library/alpine:3.21";
    assert!(h1_prepare(&s, next, true).await.is_err());
    h1_assert_current(&s, H1_B);
    assert!(!crate::cli::rootfs_cache::cache_candidates(&h1_cache(&s), next)[0].exists());
}

#[tokio::test]
async fn h1_changed_pin_cannot_upgrade_an_old_shared_strict_root_to_a_writer() {
    let s = setup(false);
    valid_project(&s);
    let a = h1_prepare(&s, H1_A, true).await.unwrap();
    h1_retire(&s, &a).await;
    let pin = format!("{H1_A}@sha256:{}", "c".repeat(64));
    let before = std::fs::read(&a.lower).unwrap();
    let result = h1_prepare(&s, &pin, true).await;
    assert!(
        result.is_err(),
        "miss preparation must not upgrade old Strict candidates"
    );
    assert_eq!(std::fs::read(&a.lower).unwrap(), before);
    h1_assert_current(&s, H1_A);
    a.validate(H1_A).unwrap();
    assert!(!crate::cli::rootfs_cache::cache_candidates(&h1_cache(&s), &pin)[1].exists());
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
    let local = test_source_command(context(&s), "w", &s.source, None, over())
        .await
        .err()
        .unwrap();
    assert!(local.contains("compatibility custody"));
    let bundle = test_source_command(context(&s), "w", &s.source, Some("pkg"), over())
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
        test_binary_command(context(&s), "pkg", "bin/w", "w", &s.source, binary)
            .await
            .err()
            .unwrap()
            .contains("compatibility custody")
    );
    assert!(
        test_build_command(context(&s), &s.source, &s.vm.join("build-1"))
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
        let local = test_source_command(context(&s), "w", &s.source, None, over())
            .await
            .err()
            .unwrap();
        assert!(local.contains("compatibility custody"), "{local}");
        let bundle = test_source_command(context(&s), "w", &s.source, Some("pkg"), over())
            .await
            .err()
            .unwrap();
        assert!(bundle.contains("compatibility custody"), "{bundle}");
        let binary = test_binary_command(context(&s), "pkg", "missing", "w", &s.source, over())
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
    let error = test_build_command(context(&s), &original, &scratch)
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

// REQUIRED PUBLIC NORMAL-MODE MATRIX, BLOCKED on the reviewed C1-C6 protocol.
// These are acceptance specifications, NOT executed tests or helper-only proof:
// - WorkerVms::build with valid local manifest/base_image and scoped B claims an
//   existing output under B. A's public build of that same output must be S505,
//   with A's claim/slots/counters and B's image/id/partials unchanged. An alias
//   spelling must refuse in input admission (S507), without changing either claim.
// - Public build with valid source and absent generic out + out/image.erofs/id
//   must retain the same enrollment fence through creation, build VM and writer;
//   retry as A against B's recorded-but-absent output must remain S505 with no mkdir.
// - Successful first build, cache hit and rebuild must preserve existing hook,
//   image/id publication and cleanup semantics, including normal return values.
// - Cover both source-supplied and request-overridden base images, and direct
//   project token versus scoped token; rejected input/cache must still precede
//   all claims/output/sweeps/firmware/channel work.
// Do NOT make these pass with a no-op checkpoint, prepared global HOME, real
// network/VM fixtures here, identity refresh, or an unadmitted convenience mkdir.
// This source-only round tests the real checked lock/cache control flow with an
// own-temp path and a deterministic pull future, not the full public build flow.

#[tokio::test]
async fn public_start_refuses_another_scoped_claim_before_firmware_with_real_context() {
    // This public build-start path requires the integration profile with overlay.
    // Never turn an unsupported test profile into a passing normal-mode claim.
    assert!(
        crate::cli::overlay::overlay_active(),
        "run with the admitted overlay test profile"
    );
    let s = setup(false);
    valid_project(&s);
    let dir = s._root.path().join("claimed-image");
    private(&dir);
    std::fs::write(dir.join(IMAGE_FILE), b"fixture-image-not-booted").unwrap();
    std::fs::write(dir.join(IMAGE_ID_FILE), b"sha256:fixture\n").unwrap();
    let other = crate::wire::scoped_auth_token("token", "other");
    s.vms
        .claim_build_dir(&other, "other", &dir, "output_dir")
        .unwrap();
    let mut req = start(&s);
    req.auth = crate::wire::scoped_auth_token("token", "key");
    req.build = Some(WorkerBuildRef {
        build_dir: dir.display().to_string(),
        build_id: "sha256:fixture".into(),
    });
    let error = s.vms.start(req).await.unwrap_err();
    assert!(matches!(error, SandboxError::BuildDirRefused(_)), "{error}");
    assert!(!s.vm.join(BUILD_DIR_RECORD).exists());
    assert!(s.vms.slots().is_empty());
    assert_eq!(
        std::fs::read(dir.join(IMAGE_FILE)).unwrap(),
        b"fixture-image-not-booted"
    );
    assert_eq!(
        std::fs::read(s.vm.join("state.json")).unwrap(),
        b"{\"instance\":7}"
    );
}

async fn install_retired_fixture(s: &Setup) {
    let lease = s.vms.admission(Some("key")).unwrap();
    let session = crate::sandbox_daemon::adapters::runtime_test_session(
        lease.clone(),
        &s._root.path().join("catalog"),
        false,
    )
    .await;
    let (status, receipt) = session.wait().await;
    let receipt = receipt.unwrap();
    session.finish_resources(&receipt).await.unwrap();
    let console = Console {
        admission: Some(lease.clone()),
        failed: false,
        dir: s.vm.clone(),
        file: File::open(s.vm.join(CONSOLE_LOG)).unwrap(),
        len: 12,
        base: 0,
    };
    let vm = Arc::new(Vm {
        instance: 7,
        pid: Pid::from_raw(0),
        lifecycle: Some(session),
        admission: lease,
        retirement: Mutex::new(Some(Ok(receipt))),
        started_at_ms: 0,
        stopping: AtomicBool::new(false),
        reaped: AtomicBool::new(true),
        ended: watch::Sender::new(Some(Ended {
            exit_code: status.and_then(|s| s.code()),
            signal: None,
            at_ms: now_ms(),
        })),
        console: Arc::new(Mutex::new(console)),
        followers: watch::Sender::new(0),
        leased: AtomicBool::new(false),
    });
    s.vms.slots().insert(vm_id("key"), Slot::Booted(vm));
}

#[tokio::test]
async fn admitted_destroy_retains_guest_data_then_explicit_prune_removes_only_vm_tree() {
    let s = setup(false);
    let backup = s._root.path().join("independent-backup");
    private(&backup);
    std::fs::write(backup.join("sentinel"), b"protected").unwrap();
    install_retired_fixture(&s).await;
    let retained = s
        .vms
        .destroy(WorkerDestroyRequest {
            auth: "token".into(),
            state_key: "key".into(),
            prune: false,
        })
        .unwrap();
    assert!(retained.destroyed);
    assert!(retained.kept.iter().any(|p| p.contains(PRE_BUILD_DISK)));
    assert_eq!(
        std::fs::read(s.vm.join(PRE_BUILD_DISK)).unwrap(),
        b"OLD-do-not-archive"
    );
    assert!(!s.vm.join(STATE_FILE).exists());
    assert!(s.vm.join(BUILD_MODE_SEAL).exists());
    assert!(s.vm.exists());
    // Prune is tested as the first removal of a different genuine lifecycle.
    let fresh = setup(false);
    install_retired_fixture(&fresh).await;
    let pruned = fresh
        .vms
        .destroy(WorkerDestroyRequest {
            auth: "token".into(),
            state_key: "key".into(),
            prune: true,
        })
        .unwrap();
    assert!(pruned.destroyed);
    assert!(!fresh.vm.exists());
    assert_eq!(
        std::fs::read(s.vm.join(PRE_BUILD_DISK)).unwrap(),
        b"OLD-do-not-archive"
    );
    assert_eq!(
        std::fs::read(backup.join("sentinel")).unwrap(),
        b"protected"
    );
}

#[tokio::test]
async fn consumed_destroy_receipt_cannot_be_expanded_into_later_prune() {
    let s = setup(false);
    install_retired_fixture(&s).await;
    s.vms
        .destroy(WorkerDestroyRequest {
            auth: "token".into(),
            state_key: "key".into(),
            prune: false,
        })
        .unwrap();
    assert!(
        s.vms
            .destroy(WorkerDestroyRequest {
                auth: "token".into(),
                state_key: "key".into(),
                prune: true
            })
            .is_err()
    );
    assert_eq!(
        std::fs::read(s.vm.join(PRE_BUILD_DISK)).unwrap(),
        b"OLD-do-not-archive"
    );
}
