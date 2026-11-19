// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (R7). Real core methods only: actual Create/ControlFiles/
//! ImagePair mutations, a real runtime spawn plus owned `wait` retirement (the
//! runtime OFD is activated in-process exactly like the existing runtime wait
//! fixture), real H/H1 probe and handoff, and real one-shot Remove. Positive
//! paths never forge a receipt, completion, journal or pin.
use super::*;
use crate::worker_admission::{
    CacheEntry, Lease, LogSpec, MutationPlan, PathRequest, WriterRetirement, testing, with_paths,
};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
struct Fixture {
    _temp: tempfile::TempDir,
    context: crate::worker_admission::AdmissionContext,
    lease: Lease,
    data: PathBuf,
    a: PathBuf,
    b: PathBuf,
    out: PathBuf,
    image: PathBuf,
    id: PathBuf,
    spool: PathBuf,
    log: PathBuf,
    old_log: PathBuf,
}
fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn populate(root: &Path) {
    fs::create_dir(root).unwrap();
    fs::write(root.join("content"), b"SOURCE").unwrap();
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    fs::write(root.with_file_name(format!("{name}.erofs")), b"LOWER").unwrap();
    fs::write(
        root.with_file_name(format!("{name}.digest")),
        b"sha256:fixture",
    )
    .unwrap();
}
/// Same declaration shape as the build: the output tree plus its exact
/// `image.erofs`/`image.id`/`image.spool` slots, and a console log pair. Cache
/// entries and the log directory exist BEFORE the original Lease; the output
/// root and the log files are created by owned mutations before retirement.
fn fixture() -> Fixture {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    let data = temp.path().join("data");
    private(&ns);
    private(&data);
    let context = testing::provision_context(&ns).unwrap();
    let cache = data.join("cache");
    private(&cache);
    let a = cache.join("a");
    let b = cache.join("b");
    populate(&a);
    populate(&b);
    let logs = data.join("logs");
    private(&logs);
    let out = data.join("output");
    let image = out.join("image.erofs");
    let id = out.join("image.id");
    let spool = out.join("image.spool");
    let log = logs.join("console.log");
    let old_log = logs.join("console.log.1");
    let original = with_paths(
        context.local(&data.join("vm")).unwrap(),
        &[
            PathRequest {
                path: &out,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &image,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &id,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &spool,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &old_log,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    // H: the original lifecycle selects cache A before any runtime exists.
    let lease = original
        .probe_cache(&[CacheEntry { root: &a }])
        .unwrap()
        .handoff(&original, &a)
        .unwrap();
    assert!(Arc::ptr_eq(&lease.0, &original.0));
    drop(original);
    let mut create = lease
        .begin(MutationPlan::Create { root: out.clone() })
        .unwrap();
    create.create_dir_all(&out, 0o700).unwrap();
    create.commit().unwrap();
    let mut init = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![log.clone(), old_log.clone()],
        })
        .unwrap();
    for path in [&log, &old_log] {
        let stage = init.stage_file(path, 0o600).unwrap();
        init.publish_file(stage).unwrap();
    }
    init.commit().unwrap();
    Fixture {
        _temp: temp,
        context,
        lease,
        data,
        a,
        b,
        out,
        image,
        id,
        spool,
        log,
        old_log,
    }
}
fn simple_command(script: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg(script);
    command
}
/// Real spawn and owned `wait`; the receipt is issued by the core, not here.
async fn retire(lease: &Lease) -> WriterRetirement {
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    child.activate_own_context_for_test().unwrap();
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    completion.retirement().unwrap().clone()
}
/// Mirrors worker_vm `ImageWrite::write_admitted` step for step through the
/// real core methods: stage image, id and spool; writer clones own the image
/// and spool bytes and are dropped (joined) before the spool discard and the
/// two publications; the id names the completed image digest.
fn write_pair(lease: &Lease, out: &Path, bytes: &[u8]) -> String {
    let mut guard = lease
        .begin(MutationPlan::ImagePair {
            directory: out.to_path_buf(),
        })
        .unwrap();
    let mut image = guard.stage_file(&out.join("image.erofs"), 0o600).unwrap();
    let mut id = guard.stage_file(&out.join("image.id"), 0o600).unwrap();
    let mut spool = guard.stage_file(&out.join("image.spool"), 0o600).unwrap();
    {
        let mut writer = image.file_mut().try_clone().unwrap();
        let mut temporary = spool.file_mut().try_clone().unwrap();
        temporary.write_all(b"spooled tar input").unwrap();
        writer.write_all(bytes).unwrap();
    }
    let build_id = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    id.file_mut()
        .write_all(format!("{build_id}\n").as_bytes())
        .unwrap();
    image.file_mut().sync_all().unwrap();
    id.file_mut().sync_all().unwrap();
    guard.discard_file(spool).unwrap();
    guard.publish_file(image).unwrap();
    guard.publish_file(id).unwrap();
    guard.commit().unwrap();
    build_id
}
fn pair_journal(f: &Fixture) -> Journal {
    let tx = f.context.namespace.transaction().unwrap();
    tx.operation(&f.lease.0.id)
        .unwrap()
        .journals
        .iter()
        .rev()
        .find(|j| j.plan == "image-pair")
        .unwrap()
        .clone()
}
/// H1 with warm cache B through the real probe/read/handoff path.
fn continue_to_b(f: &Fixture, receipt: &WriterRetirement) -> io::Result<Lease> {
    let probe = receipt.probe_cache(&f.lease, &[CacheEntry { root: &f.b }])?;
    assert!(probe.exists(&f.b)?);
    let read = probe.use_path(&f.b)?;
    read.revalidate()?;
    drop(read);
    probe.handoff(&f.lease, &f.b)
}
#[tokio::test]
async fn r7_actual_wait_then_exact_image_pair_allows_h1_and_preserves_a_digest_and_arc() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    let bytes = b"EROFS-fixture-bytes";
    let build_id = write_pair(&f.lease, &f.out, bytes);
    // The committed journal carries the structural proof derived from the
    // recorded completions, and nothing else.
    let journal = pair_journal(&f);
    let recorded = journal.image_pair.as_ref().expect("exact pair proof");
    assert_eq!(
        crate::worker_admission::recovery::digest(recorded).unwrap(),
        crate::worker_admission::recovery::digest(&proof(&journal).unwrap().unwrap()).unwrap()
    );
    assert_eq!(journal.stage_completions.len(), 3);
    let lease_b = continue_to_b(&f, &receipt).unwrap();
    assert!(Arc::ptr_eq(&lease_b.0, &f.lease.0));
    // Old cache claim A survives; B is newly claimed; outsiders cannot take A.
    lease_b.use_path(&f.a).unwrap().revalidate().unwrap();
    lease_b.use_path(&f.b).unwrap().revalidate().unwrap();
    let outsider = f.context.local(&f.data.join("other")).unwrap();
    assert!(
        outsider
            .prepare_cache(&[CacheEntry { root: &f.a }])
            .is_err()
    );
    // The published pair is exactly the completed bytes and their digest.
    assert_eq!(fs::read(&f.image).unwrap(), bytes);
    assert_eq!(fs::read_to_string(&f.id).unwrap(), format!("{build_id}\n"));
    assert!(!f.spool.exists());
}
#[tokio::test]
async fn r7_actual_wait_then_exact_image_pair_allows_owned_cleanup_lineage() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    write_pair(&f.lease, &f.out, b"cleanup-bytes");
    let mut removal = receipt
        .begin_remove(&f.lease, &f.out, &[&f.image, &f.id])
        .unwrap();
    removal.remove_path(&f.image).unwrap();
    removal.remove_path(&f.id).unwrap();
    removal.commit().unwrap();
    assert!(!f.image.exists() && !f.id.exists());
    assert!(f.out.exists());
    assert!(f.a.exists() && f.b.exists());
}
#[tokio::test]
async fn r7_rebuild_over_owned_pair_keeps_priors_and_continues_exact_lineage() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    write_pair(&f.lease, &f.out, b"first-build");
    let second = write_pair(&f.lease, &f.out, b"second-build");
    continue_to_b(&f, &receipt).unwrap();
    assert_eq!(fs::read(&f.image).unwrap(), b"second-build");
    assert_eq!(fs::read_to_string(&f.id).unwrap(), format!("{second}\n"));
    // The old generation is preserved as evidence, never deleted by the pair.
    let priors = fs::read_dir(&f.out)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().starts_with(".iii-prior-"))
        .count();
    assert_eq!(priors, 2);
}
#[tokio::test]
async fn r7_finished_log_after_wait_coexists_with_exact_pair_but_not_altered_image() {
    for altered in [false, true] {
        let f = fixture();
        let writer = f
            .lease
            .open_log(LogSpec {
                current: &f.log,
                previous: Some(&f.old_log),
                cap_bytes: 1024,
            })
            .unwrap();
        writer.append(b"runtime line\n").unwrap();
        let receipt = retire(&f.lease).await;
        write_pair(&f.lease, &f.out, b"actual pair bytes");
        // Host pump after the owned wait, then the sole writer finishes.
        writer.append(b"last pump record\n").unwrap();
        writer.finish().unwrap();
        if altered {
            fs::rename(&f.image, f.out.join("held-original")).unwrap();
            fs::write(&f.image, b"not original").unwrap();
        }
        // A successful log finish neither substitutes for the pair proof nor
        // refreshes the image identity.
        assert_eq!(continue_to_b(&f, &receipt).is_ok(), !altered);
        if altered {
            assert_eq!(fs::read(&f.image).unwrap(), b"not original");
            assert!(receipt.begin_remove(&f.lease, &f.out, &[&f.image]).is_err());
            assert_eq!(fs::read(&f.image).unwrap(), b"not original");
        }
    }
}
#[tokio::test]
async fn r7_external_imagepair_replacement_is_not_blessed_by_the_old_retirement() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    write_pair(&f.lease, &f.out, b"owned image");
    fs::rename(&f.image, f.out.join("held-old")).unwrap();
    fs::write(&f.image, b"external replacement").unwrap();
    assert!(continue_to_b(&f, &receipt).is_err());
    {
        // B was never claimed by the original lifecycle; A still is.
        let tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation(&f.lease.0.id).unwrap();
        let b = HostPath::new(&f.b).unwrap();
        assert!(!op.entries.iter().any(|e| e.path == b));
    }
    f.lease.use_path(&f.a).unwrap().revalidate().unwrap();
    assert!(
        receipt
            .begin_remove(&f.lease, &f.out, &[&f.image, &f.id])
            .is_err()
    );
    assert_eq!(fs::read(&f.image).unwrap(), b"external replacement");
    assert_eq!(fs::read(f.out.join("held-old")).unwrap(), b"owned image");
}
#[tokio::test]
async fn r7_partial_pair_without_id_publication_gets_no_post_retirement_authority() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    let mut guard = f
        .lease
        .begin(MutationPlan::ImagePair {
            directory: f.out.clone(),
        })
        .unwrap();
    let mut image = guard.stage_file(&f.image, 0o600).unwrap();
    let spool = guard.stage_file(&f.spool, 0o600).unwrap();
    image.file_mut().write_all(b"image without id").unwrap();
    guard.discard_file(spool).unwrap();
    guard.publish_file(image).unwrap();
    // A generic ImagePair commit stays valid but carries no pair authority.
    guard.commit().unwrap();
    assert!(pair_journal(&f).image_pair.is_none());
    assert!(continue_to_b(&f, &receipt).is_err());
    assert!(receipt.begin_remove(&f.lease, &f.out, &[&f.image]).is_err());
    assert_eq!(fs::read(&f.image).unwrap(), b"image without id");
}
#[tokio::test]
async fn r7_wrong_id_fails_commit_and_never_advances_the_receipt() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    let mut guard = f
        .lease
        .begin(MutationPlan::ImagePair {
            directory: f.out.clone(),
        })
        .unwrap();
    let mut image = guard.stage_file(&f.image, 0o600).unwrap();
    let mut id = guard.stage_file(&f.id, 0o600).unwrap();
    let spool = guard.stage_file(&f.spool, 0o600).unwrap();
    image.file_mut().write_all(b"image").unwrap();
    id.file_mut()
        .write_all(b"sha256:not-the-image-digest\n")
        .unwrap();
    guard.discard_file(spool).unwrap();
    guard.publish_file(image).unwrap();
    guard.publish_file(id).unwrap();
    assert!(guard.commit().is_err());
    assert!(
        f.context
            .namespace
            .transaction()
            .unwrap()
            .operation(&f.lease.0.id)
            .unwrap()
            .uncertain
    );
    assert!(continue_to_b(&f, &receipt).is_err());
}
#[tokio::test]
async fn r7_missing_or_tampered_pair_proof_is_refused_by_h1_and_cleanup() {
    for missing in [true, false] {
        let f = fixture();
        let receipt = retire(&f.lease).await;
        write_pair(&f.lease, &f.out, b"image");
        {
            let mut tx = f.context.namespace.transaction().unwrap();
            let journal = tx
                .operation_mut(&f.lease.0.id)
                .unwrap()
                .journals
                .iter_mut()
                .rev()
                .find(|j| j.plan == "image-pair")
                .unwrap();
            if missing {
                journal.image_pair = None;
            } else {
                journal.stage_completions[1].sha256 = "0".repeat(64);
            }
            tx.save().unwrap();
        }
        assert!(continue_to_b(&f, &receipt).is_err());
        assert!(
            receipt
                .begin_remove(&f.lease, &f.out, &[&f.image, &f.id])
                .is_err()
        );
        assert!(f.image.exists() && f.id.exists());
    }
}
#[test]
fn r7_live_writer_clone_prevents_pair_publication_and_discard() {
    for image_live in [false, true] {
        let f = fixture();
        let mut guard = f
            .lease
            .begin(MutationPlan::ImagePair {
                directory: f.out.clone(),
            })
            .unwrap();
        let mut stage = guard
            .stage_file(if image_live { &f.image } else { &f.spool }, 0o600)
            .unwrap();
        let path = stage.path().to_path_buf();
        // An un-joined writer clone still shares the stage OFD lock.
        let writer = stage.file_mut().try_clone().unwrap();
        let result = if image_live {
            guard.publish_file(stage)
        } else {
            guard.discard_file(stage)
        };
        assert!(result.is_err());
        assert!(path.exists());
        assert!(!f.image.exists());
        drop(writer);
        assert!(guard.commit().is_err());
        assert!(path.exists());
    }
}
#[tokio::test]
async fn r7_new_mutable_target_or_root_after_retirement_is_not_blessed() {
    let f = fixture();
    let receipt = retire(&f.lease).await;
    let original = {
        let tx = f.context.namespace.transaction().unwrap();
        tx.operation(&f.lease.0.id).unwrap().entries.clone()
    };
    write_pair(&f.lease, &f.out, b"image");
    // A new root declared after retirement, with a complete exact pair in it.
    let other = f.data.join("other-output");
    let lease = with_paths(
        f.lease.clone(),
        &[
            PathRequest {
                path: &other,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &other.join("image.erofs"),
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &other.join("image.id"),
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &other.join("image.spool"),
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let mut create = lease
        .begin(MutationPlan::Create {
            root: other.clone(),
        })
        .unwrap();
    create.create_dir_all(&other, 0o700).unwrap();
    create.commit().unwrap();
    write_pair(&lease, &other, b"late image");
    let late = pair_journal(&f);
    assert_eq!(late.root, HostPath::new(&other).unwrap());
    // The exact proof exists, but its slots are not in the original receipt.
    assert!(late.image_pair.is_some());
    assert!(validate_lineage(&late, &original).is_err());
    assert!(continue_to_b(&f, &receipt).is_err());
    // Same for one late mutable file under the original root.
    let f = fixture();
    let receipt = retire(&f.lease).await;
    write_pair(&f.lease, &f.out, b"image");
    let later = f.out.join("later");
    let lease = with_paths(
        f.lease.clone(),
        &[PathRequest {
            path: &later,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    assert!(continue_to_b(&f, &receipt).is_err());
    assert!(receipt.begin_remove(&lease, &f.out, &[&later]).is_err());
}
#[test]
fn r7_structural_proof_rejects_reordered_unbound_or_extraneous_chains() {
    let f = fixture();
    write_pair(&f.lease, &f.out, b"image");
    let journal = pair_journal(&f);
    let entries = {
        let tx = f.context.namespace.transaction().unwrap();
        tx.operation(&f.lease.0.id).unwrap().entries.clone()
    };
    assert!(proof(&journal).unwrap().is_some());
    validate_lineage(&journal, &entries).unwrap();
    let mut reordered = journal.clone();
    reordered.stage_completions.swap(1, 2);
    assert!(proof(&reordered).is_err());
    let mut unbound = journal.clone();
    unbound.stage_completions[0].identity.ino ^= 1;
    assert!(proof(&unbound).is_err());
    let mut undiscarded = journal.clone();
    undiscarded.disposals[0].discarded = false;
    assert!(proof(&undiscarded).is_err());
    let mut extraneous = journal.clone();
    let mut extra = journal.intents[0].clone();
    extra.kind = EffectKind::Create;
    extra.to = HostPath::new(&f.out.join("nested")).unwrap();
    extraneous.intents.push(extra);
    assert!(proof(&extraneous).is_err());
    // Without the spool disposal it is a generic ImagePair: no authority.
    let mut generic = journal.clone();
    generic.disposals.clear();
    assert!(proof(&generic).unwrap().is_none());
    // The derived proof alone is not enough: the recorded receipt is required.
    let mut unrecorded = journal.clone();
    unrecorded.image_pair = None;
    assert!(validate_lineage(&unrecorded, &entries).is_err());
}
