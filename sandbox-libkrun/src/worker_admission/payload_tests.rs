// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN: representative payload tests require admitted FD/disk budgets.
use super::*;
use crate::worker_admission::{MutationPlan, PathRequest, PathRole, testing, with_paths};
use std::{
    fs,
    io::Cursor,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
struct Fixture {
    _temp: tempfile::TempDir,
    lease: Lease,
    data: PathBuf,
    target: PathBuf,
}
fn fixture() -> Fixture {
    fixture_with(|_| ())
}
/// Fixture objects get exact modes, never the process umask.
fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn dir_mode(path: &Path, mode: u32) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
/// `existing` creates prior target data BEFORE the original Lease is issued,
/// so admission pins it as preexisting state instead of refusing a path that
/// appeared outside an owned mutation. Never used to refresh a recorded path.
fn fixture_with(existing: impl FnOnce(&Path)) -> Fixture {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    fs::create_dir(&ns).unwrap();
    fs::set_permissions(&ns, fs::Permissions::from_mode(0o700)).unwrap();
    let context = testing::provision_context(&ns).unwrap();
    let data = temp.path().to_path_buf();
    let target = data.join("payload");
    existing(&target);
    let lease = context.local(&target).unwrap();
    Fixture {
        _temp: temp,
        lease,
        data,
        target,
    }
}
fn metadata(mode: u32) -> PayloadMetadata {
    PayloadMetadata {
        mode,
        uid: uid(),
        gid: unsafe { libc::getegid() },
        atime_sec: 1700000000,
        atime_nsec: 123,
        mtime_sec: 1700000001,
        mtime_nsec: 456,
        xattrs: BTreeMap::new(),
    }
}
fn entry(path: &str, mode: u32, kind: PayloadKind) -> PayloadEntry {
    PayloadEntry {
        relative: PathBuf::from(path),
        metadata: metadata(mode),
        kind,
    }
}
#[test]
fn payload_common_0755_root_1777_tmp_0777_child_and_internal_hardlink_are_faithful() {
    let f = fixture();
    let plan = PayloadInventory::new(vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("tmp", 0o1777, PayloadKind::Directory),
        entry("shared", 0o777, PayloadKind::Directory),
        entry("data", 0o644, PayloadKind::File { size: 4 }),
        entry(
            "link",
            0o644,
            PayloadKind::Hardlink {
                source: PathBuf::from("data"),
            },
        ),
    ])
    .unwrap();
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    let mut producer = guard.produce_payload(&f.target, plan).unwrap();
    producer
        .write_file(Path::new("data"), &mut Cursor::new(b"DATA"))
        .unwrap();
    producer.seal().unwrap().publish().unwrap();
    assert_eq!(fs::metadata(&f.target).unwrap().mode() & 0o7777, 0o755);
    assert_eq!(
        fs::metadata(f.target.join("tmp")).unwrap().mode() & 0o7777,
        0o1777
    );
    assert_eq!(
        fs::metadata(f.target.join("shared")).unwrap().mode() & 0o7777,
        0o777
    );
    assert_eq!(fs::read(f.target.join("link")).unwrap(), b"DATA");
    assert_eq!(
        fs::metadata(f.target.join("data")).unwrap().ino(),
        fs::metadata(f.target.join("link")).unwrap().ino()
    );
    assert_eq!(fs::metadata(f.target.join("data")).unwrap().nlink(), 2);
    f.lease.revalidate().unwrap();
}
#[test]
fn payload_inventory_above_4096_entries_is_admitted_and_published_without_limit_bump() {
    let f = fixture();
    let mut entries = vec![entry("", 0o755, PayloadKind::Directory)];
    for n in 0..5000 {
        entries.push(entry(
            &format!("f{n:05}"),
            0o644,
            PayloadKind::File { size: 1 },
        ));
    }
    let inventory = PayloadInventory::new(entries).unwrap();
    assert_eq!(inventory.entries.len(), 5001);
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    let mut producer = guard
        .produce_payload(&f.target, inventory)
        .expect("test workload must admit actual FD/inode/disk inventory; Unsupported is not PASS");
    for n in 0..5000 {
        producer
            .write_file(Path::new(&format!("f{n:05}")), &mut Cursor::new(b"x"))
            .unwrap();
    }
    producer.seal().unwrap().publish().unwrap();
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 5000);
}
#[test]
fn payload_readable_clone0755_uses_core_writer_and_preserves_metadata() {
    let f = fixture();
    let source = f.data.join("source");
    fs::create_dir(&source).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir(source.join("tmp")).unwrap();
    fs::set_permissions(source.join("tmp"), fs::Permissions::from_mode(0o1777)).unwrap();
    write_mode(&source.join("data"), b"DATA", 0o644);
    fs::hard_link(source.join("data"), source.join("hard")).unwrap();
    let lease = with_paths(
        f.lease.clone(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let readable = lease.capture_payload(&source).unwrap();
    let guard = lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    readable
        .produce(guard, &f.target)
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(fs::metadata(&f.target).unwrap().mode() & 0o7777, 0o755);
    assert_eq!(
        fs::metadata(f.target.join("tmp")).unwrap().mode() & 0o7777,
        0o1777
    );
    assert_eq!(fs::metadata(f.target.join("hard")).unwrap().nlink(), 2);
}
#[test]
fn payload_incomplete_writer_never_emits_seal_and_preserves_existing_target() {
    // The existing target is prior data the original Lease pins, not a path
    // that appeared after admission recorded it absent.
    let f = fixture_with(|target| {
        fs::create_dir(target).unwrap();
        fs::set_permissions(target, fs::Permissions::from_mode(0o755)).unwrap();
        write_mode(&target.join("guest-data"), b"KEEP", 0o644);
    });
    let inventory = PayloadInventory::new(vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("missing", 0o644, PayloadKind::File { size: 4 }),
    ])
    .unwrap();
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    let producer = guard.produce_payload(&f.target, inventory).unwrap();
    assert!(producer.seal().is_err());
    assert_eq!(fs::read(f.target.join("guest-data")).unwrap(), b"KEEP");
    assert!(f.lease.revalidate().is_err());
}
#[test]
fn payload_writer_short_extra_and_second_write_are_terminal() {
    for size in [2, 6] {
        let f = fixture();
        let inventory = PayloadInventory::new(vec![
            entry("", 0o755, PayloadKind::Directory),
            entry("data", 0o644, PayloadKind::File { size }),
        ])
        .unwrap();
        let guard = f
            .lease
            .begin(MutationPlan::Reclone {
                root: f.target.clone(),
            })
            .unwrap();
        let mut producer = guard.produce_payload(&f.target, inventory).unwrap();
        assert!(
            producer
                .write_file(Path::new("data"), &mut Cursor::new(b"DATA"))
                .is_err()
        );
        assert!(
            producer
                .write_file(Path::new("data"), &mut Cursor::new(b"DATA"))
                .is_err()
        );
        assert!(producer.seal().is_err());
        assert!(!f.target.exists());
    }
}
#[test]
fn payload_seal_rejects_stage_or_member_replacement_instead_of_refreshing_identity() {
    let f = fixture();
    let inventory = PayloadInventory::new(vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("data", 0o644, PayloadKind::File { size: 4 }),
    ])
    .unwrap();
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    let mut producer = guard.produce_payload(&f.target, inventory).unwrap();
    producer
        .write_file(Path::new("data"), &mut Cursor::new(b"DATA"))
        .unwrap();
    let seal = producer.seal().unwrap();
    let path = seal.builder.stage.join("data");
    fs::rename(&path, seal.builder.stage.join("retained")).unwrap();
    fs::write(&path, b"FAKE").unwrap();
    assert!(seal.publish().is_err());
    assert!(!f.target.exists());
}
#[test]
fn payload_path_inventory_and_hardlink_outside_scope_are_rejected_before_effects() {
    assert!(
        PayloadInventory::new(vec![
            entry("", 0o755, PayloadKind::Directory),
            entry("../escape", 0o644, PayloadKind::File { size: 1 })
        ])
        .is_err()
    );
    assert!(
        PayloadInventory::new(vec![
            entry("", 0o755, PayloadKind::Directory),
            entry(
                "link",
                0o644,
                PayloadKind::Hardlink {
                    source: PathBuf::from("../foreign")
                }
            )
        ])
        .is_err()
    );
    let f = fixture();
    let other = f.data.join("other");
    let inventory = PayloadInventory::new(vec![entry("", 0o755, PayloadKind::Directory)]).unwrap();
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    assert!(guard.produce_payload(&other, inventory).is_err());
    assert!(!other.exists());
}

#[test]
fn payload_cache_entry_seals_source_then_digest_commits_in_same_guard() {
    use crate::worker_admission::CacheEntry;
    use std::io::Write;
    let f = fixture();
    let root = f.data.join("image");
    let prep = f
        .lease
        .prepare_cache(&[CacheEntry { root: &root }])
        .unwrap();
    let inventory = PayloadInventory::new(vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("tmp", 0o1777, PayloadKind::Directory),
        entry("file", 0o644, PayloadKind::File { size: 4 }),
    ])
    .unwrap();
    let guard = prep.begin_entry(&root).unwrap();
    let mut builder = guard.produce_payload(&root, inventory).unwrap();
    builder
        .write_file(Path::new("file"), &mut Cursor::new(b"DATA"))
        .unwrap();
    let mut guard = builder.seal().unwrap().publish_into_guard().unwrap();
    let digest = f.data.join("image.digest");
    let mut stage = guard.stage_file(&digest, 0o600).unwrap();
    stage.file_mut().write_all(b"digest").unwrap();
    guard.publish_file(stage).unwrap();
    guard.commit().unwrap();
    assert_eq!(
        fs::metadata(root.join("tmp")).unwrap().mode() & 0o7777,
        0o1777
    );
    assert_eq!(fs::read(digest).unwrap(), b"digest");
}
#[test]
fn payload_source_external_link_and_source_mutation_do_not_create_seals() {
    let f = fixture();
    let source = f.data.join("source");
    dir_mode(&source, 0o755);
    write_mode(&source.join("data"), b"DATA", 0o644);
    fs::hard_link(source.join("data"), f.data.join("external")).unwrap();
    let lease = with_paths(
        f.lease.clone(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    assert!(lease.capture_payload(&source).is_err());
    assert!(!f.target.exists());
    let f = fixture();
    let source = f.data.join("source");
    dir_mode(&source, 0o755);
    write_mode(&source.join("data"), b"DATA", 0o644);
    let lease = with_paths(
        f.lease.clone(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let captured = lease.capture_payload(&source).unwrap();
    fs::write(source.join("data"), b"EVIL").unwrap();
    let guard = lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    assert!(captured.produce(guard, &f.target).is_err());
    assert!(!f.target.exists());
}

#[test]
fn payload_metadata_and_seal_faults_never_publish_or_delete_prior_data() {
    for at in ["metadata", "before-seal"] {
        // Prior data exists BEFORE the original Lease records the target.
        let f = fixture_with(|target| {
            fs::create_dir(target).unwrap();
            fs::set_permissions(target, fs::Permissions::from_mode(0o755)).unwrap();
            write_mode(&target.join("old"), b"KEEP", 0o644);
        });
        let inventory = PayloadInventory::new(vec![
            entry("", 0o755, PayloadKind::Directory),
            entry("tmp", 0o1777, PayloadKind::Directory),
            entry("file", 0o644, PayloadKind::File { size: 1 }),
        ])
        .unwrap();
        let guard = f
            .lease
            .begin(MutationPlan::Reclone {
                root: f.target.clone(),
            })
            .unwrap();
        let mut builder = guard.produce_payload(&f.target, inventory).unwrap();
        builder
            .write_file(Path::new("file"), &mut Cursor::new(b"x"))
            .unwrap();
        builder.fault(at);
        assert!(builder.seal().is_err());
        assert_eq!(fs::read(f.target.join("old")).unwrap(), b"KEEP");
        assert!(f.lease.revalidate().is_err());
    }
}
#[test]
fn payload_local_tar_stream_metadata_is_applied_by_core_not_external_stage_writer() {
    // Local tar is input only. The final merged inventory is explicit; no path
    // based extractor writes into the sealed stage and no bool claims a join.
    let f = fixture();
    let mut tar_bytes = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_uid(uid() as u64);
        header.set_gid(unsafe { libc::getegid() } as u64);
        header.set_cksum();
        archive
            .append_data(&mut header, "file", &b"DATA"[..])
            .unwrap();
        archive.finish().unwrap();
    }
    let inventory = PayloadInventory::new(vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("tmp", 0o1777, PayloadKind::Directory),
        entry("file", 0o644, PayloadKind::File { size: 4 }),
    ])
    .unwrap();
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    let mut builder = guard.produce_payload(&f.target, inventory).unwrap();
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    for member in archive.entries().unwrap() {
        let mut member = member.unwrap();
        let path = member.path().unwrap().into_owned();
        builder.write_file(&path, &mut member).unwrap();
    }
    builder.seal().unwrap().publish().unwrap();
    assert_eq!(fs::read(f.target.join("file")).unwrap(), b"DATA");
    assert_eq!(
        fs::metadata(f.target.join("tmp")).unwrap().mode() & 0o7777,
        0o1777
    );
}
