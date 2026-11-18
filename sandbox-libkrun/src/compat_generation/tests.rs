// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Authored offline filesystem tests, NOT old/new guest smokes. Small byte
//! fixtures deliberately do not claim to be authenticated ext4/EROFS releases.
use super::*;
use std::{
    fs as host,
    io::{Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt, symlink},
        },
    },
    path::{Path, PathBuf},
};
use store::{Outcome, Store};

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    custody: PathBuf,
    backup: PathBuf,
    managed: PathBuf,
    env: Environment,
}
fn private(path: &Path) {
    host::create_dir(path).unwrap();
    host::set_permissions(path, host::Permissions::from_mode(0o700)).unwrap();
}
fn write(path: &Path, value: &[u8]) {
    host::write(path, value).unwrap();
    host::set_permissions(path, host::Permissions::from_mode(0o600)).unwrap();
    File::open(path).unwrap().sync_all().unwrap();
}
impl Fixture {
    fn new(layout: Layout, provision: Provision, version: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        host::set_permissions(temp.path(), host::Permissions::from_mode(0o700)).unwrap();
        let source = temp.path().join("source");
        let custody = temp.path().join("custody");
        let backup = temp.path().join("offline-backup");
        let managed = temp.path().join("managed");
        for path in [&source, &custody, &backup, &managed] {
            private(path);
        }
        let lower = match layout {
            Layout::OverlayImage => {
                write(
                    &source.join("upper.ext4"),
                    b"OLD-fsynced-nonce\0opaque-whiteout-metadata-bytes",
                );
                write(&source.join("lower.erofs"), b"exact-lower-bytes");
                Some(fs::digest(b"exact-lower-bytes"))
            }
            Layout::LegacyClone => {
                private(&source.join("root"));
                private(&source.join("root/workspace"));
                write(&source.join("root/workspace/nonce"), b"OLD-fsynced-nonce");
                write(&source.join("root/os-version"), b"old-OS-keep");
                None
            }
        };
        if provision == Provision::Bundle {
            private(&source.join("bundle"));
            write(&source.join("bundle/code.js"), b"same-code-and-sdk");
            write(&source.join("bundle/data"), b"external-OLD");
        }
        let env = Environment {
            source_version: version.into(),
            layout,
            provision,
            adapter_sha256: "a".repeat(64),
            engine_sha256: "b".repeat(64),
            kernel_sha256: "c".repeat(64),
            init_sha256: "d".repeat(64),
            profile_sha256: "e".repeat(64),
            app_sha256: "f".repeat(64),
            sdk_lock_sha256: "1".repeat(64),
            provisioning_sha256: "2".repeat(64),
            lower_sha256: lower,
        };
        Self {
            _temp: temp,
            source,
            custody,
            backup,
            managed,
            env,
        }
    }
    fn grant(&self) -> OfflineGrant {
        // ONLY TESTS mint the private grant. No boolean/JSON production issuer.
        let source = fs::Dir::open(&self.source, true).unwrap();
        let lease = source.lock().unwrap();
        let pinned = tree::Inventory::scan(&source).unwrap();
        OfflineGrant {
            binding: "9".repeat(64),
            environment: self.env.clone(),
            source,
            pinned,
            _source_lease: lease,
        }
    }
    fn store<'a>(&self, grant: &'a OfflineGrant) -> Store<'a> {
        let managed = fs::Dir::open(&self.managed, true).unwrap();
        Store::open(
            grant,
            fs::Dir::open(&self.custody, true).unwrap(),
            fs::Dir::open(&self.backup, true).unwrap(),
            &[&managed],
        )
        .unwrap()
    }
    fn data(&self, operation: &str) -> PathBuf {
        self.custody.join(format!("generation-{operation}/data"))
    }
    fn nonce(&self, data: &Path) -> PathBuf {
        data.join(if self.env.layout == Layout::OverlayImage {
            "upper.ext4"
        } else {
            "root/workspace/nonce"
        })
    }
}
fn fresh() -> Fixture {
    Fixture::new(Layout::OverlayImage, Provision::Path, "0.3.1")
}

#[test]
fn eight_offline_origins_preserve_old_then_current_new_and_separate_rollback() {
    // Eight DIFFERENT source cells, not sixteen actual VM transitions. Real
    // published artifacts/guests still require two later fresh campaigns.
    for version in ["0.3.0", "0.3.1"] {
        for layout in [Layout::OverlayImage, Layout::LegacyClone] {
            for provision in [Provision::Path, Provision::Bundle] {
                let f = Fixture::new(layout.clone(), provision.clone(), version);
                let grant = f.grant();
                let store = f.store(&grant);
                assert_eq!(
                    store.capture("first", 0, &mut Faults::default()).unwrap(),
                    Outcome::Committed(1)
                );
                assert!(
                    host::read(f.nonce(&f.data("first")))
                        .unwrap()
                        .starts_with(b"OLD-fsynced-nonce")
                );
                write(&f.nonce(&f.data("first")), b"NEW-fsynced-nonce");
                if provision == Provision::Bundle {
                    write(&f.data("first").join("bundle/data"), b"external-NEW");
                }
                assert_eq!(
                    store.capture("first", 0, &mut Faults::default()).unwrap(),
                    Outcome::AlreadyCommitted(1)
                );
                assert_eq!(store.selected().unwrap().unwrap().record.generation, 1);
                assert_eq!(
                    store
                        .rebuild("second", 1, &f.env, &mut Faults::default())
                        .unwrap(),
                    Outcome::Committed(2)
                );
                assert_eq!(
                    host::read(f.nonce(&f.data("second"))).unwrap(),
                    b"NEW-fsynced-nonce"
                );
                if provision == Provision::Bundle {
                    assert_eq!(
                        host::read(f.data("second").join("bundle/data")).unwrap(),
                        b"external-NEW"
                    );
                }
                let restore = f._temp.path().join("isolated-rollback");
                private(&restore);
                store
                    .rollback_copy(
                        "first",
                        &fs::Dir::open(&restore, true).unwrap(),
                        &mut Faults::default(),
                    )
                    .unwrap();
                assert!(
                    host::read(f.nonce(&restore))
                        .unwrap()
                        .starts_with(b"OLD-fsynced-nonce")
                );
                assert_eq!(store.selected().unwrap().unwrap().record.generation, 2);
                assert_eq!(
                    host::read(f.nonce(&f.data("second"))).unwrap(),
                    b"NEW-fsynced-nonce"
                );
                assert!(
                    host::read(f.nonce(&f.source))
                        .unwrap()
                        .starts_with(b"OLD-fsynced-nonce")
                );
            }
        }
    }
}

#[test]
fn regular_metadata_xattr_hardlinks_and_deletions_are_preserved() {
    let f = Fixture::new(Layout::LegacyClone, Provision::Path, "0.3.1");
    let a = f.source.join("root/workspace/nonce");
    host::set_permissions(&a, host::Permissions::from_mode(0o640)).unwrap();
    host::hard_link(&a, f.source.join("root/workspace/alias")).unwrap();
    let file = File::open(&a).unwrap();
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                c"user.migration".as_ptr(),
                b"metadata".as_ptr().cast(),
                8,
                0,
            )
        },
        0
    );
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    let copied = f.data("first").join("root/workspace/nonce");
    let alias = f.data("first").join("root/workspace/alias");
    let m = host::metadata(&copied).unwrap();
    let old = host::metadata(&a).unwrap();
    assert_eq!(
        (m.uid(), m.gid(), m.mode() & 0o7777),
        (old.uid(), old.gid(), 0o640)
    );
    assert_eq!(m.ino(), host::metadata(alias).unwrap().ino());
    assert!(m.ino() != old.ino());
    let file = File::open(copied).unwrap();
    let mut value = [0u8; 8];
    assert_eq!(
        unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                c"user.migration".as_ptr(),
                value.as_mut_ptr().cast(),
                8,
            )
        },
        8
    );
    assert_eq!(&value, b"metadata");
    assert!(!f.data("first").join("root/workspace/deleted").exists());
}

#[test]
fn sparse_image_is_full_bytes_not_a_new_empty_upper() {
    let f = fresh();
    let mut file = host::OpenOptions::new()
        .write(true)
        .open(f.source.join("upper.ext4"))
        .unwrap();
    file.set_len(1024 * 1024).unwrap();
    file.seek(SeekFrom::Start(900_000)).unwrap();
    file.write_all(b"tail").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let grant = f.grant();
    let before = grant.pinned.file_digest(b"upper.ext4").unwrap().to_string();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    let inv = tree::Inventory::scan(&fs::Dir::open(&f.data("first"), true).unwrap()).unwrap();
    assert_eq!(inv.file_digest(b"upper.ext4"), Some(before.as_str()));
    assert_eq!(
        host::metadata(f.data("first").join("upper.ext4"))
            .unwrap()
            .len(),
        1024 * 1024
    );
}

#[test]
fn symlink_fidelity_is_supported_only_with_noatime_custody_otherwise_pure_refusal() {
    let f = Fixture::new(Layout::LegacyClone, Provision::Path, "0.3.1");
    symlink("nonce", f.source.join("root/workspace/link")).unwrap();
    let source = fs::Dir::open(&f.source, true).unwrap();
    let mut vfs = unsafe { std::mem::zeroed::<libc::statvfs>() };
    assert_eq!(unsafe { libc::fstatvfs(source.fd(), &mut vfs) }, 0);
    if vfs.f_flag & libc::ST_NOATIME == 0 {
        assert!(matches!(
            tree::Inventory::scan(&source),
            Err(Error::Unsupported(
                "symlink capture requires noatime source filesystem"
            ))
        ));
        assert_eq!(host::read_dir(&f.custody).unwrap().count(), 0);
        assert_eq!(host::read_dir(&f.backup).unwrap().count(), 0);
    } else {
        let grant = f.grant();
        let store = f.store(&grant);
        store.capture("first", 0, &mut Faults::default()).unwrap();
        assert_eq!(
            host::read_link(f.data("first").join("root/workspace/link")).unwrap(),
            PathBuf::from("nonce")
        );
    }
    // Refusal is asserted safety behavior, NOT a passed migration support cell.
}

#[test]
fn external_hardlink_is_refused_before_any_copy() {
    let f = fresh();
    host::hard_link(
        f.source.join("upper.ext4"),
        f._temp.path().join("outside-link"),
    )
    .unwrap();
    assert!(matches!(
        tree::Inventory::scan(&fs::Dir::open(&f.source, true).unwrap()),
        Err(Error::Unsupported("hardlink crosses capture boundary"))
    ));
    assert_eq!(host::read_dir(&f.backup).unwrap().count(), 0);
}
#[test]
fn fifo_special_files_refuse_before_staging() {
    let f = Fixture::new(Layout::LegacyClone, Provision::Path, "0.3.0");
    let path = std::ffi::CString::new(f.source.join("root/pipe").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(matches!(
        tree::Inventory::scan(&fs::Dir::open(&f.source, true).unwrap()),
        Err(Error::Unsupported("special file/device/socket/FIFO"))
    ));
    assert_eq!(host::read_dir(&f.custody).unwrap().count(), 0);
}
#[test]
fn lower_mismatch_and_different_os_rebuild_are_not_copies() {
    let mut f = fresh();
    f.env.lower_sha256 = Some("0".repeat(64));
    let grant = f.grant();
    let managed = fs::Dir::open(&f.managed, true).unwrap();
    assert!(
        Store::open(
            &grant,
            fs::Dir::open(&f.custody, true).unwrap(),
            fs::Dir::open(&f.backup, true).unwrap(),
            &[&managed]
        )
        .is_err()
    );
    assert_eq!(host::read_dir(&f.custody).unwrap().count(), 0);
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    let mut other = f.env.clone();
    other.profile_sha256 = "0".repeat(64);
    assert!(matches!(
        store.rebuild("other", 1, &other, &mut Faults::default()),
        Err(Error::Unsupported(_))
    ));
    assert!(!f.custody.join("stage-other").exists());
}
#[test]
fn missing_bundle_component_is_not_silently_ignored() {
    let mut f = fresh();
    f.env.provision = Provision::Bundle;
    let grant = f.grant();
    let managed = fs::Dir::open(&f.managed, true).unwrap();
    assert!(matches!(
        Store::open(
            &grant,
            fs::Dir::open(&f.custody, true).unwrap(),
            fs::Dir::open(&f.backup, true).unwrap(),
            &[&managed]
        ),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn current_new_data_survives_reopen_and_stale_capture_conflicts() {
    let f = fresh();
    let grant = f.grant();
    {
        let store = f.store(&grant);
        store.capture("first", 0, &mut Faults::default()).unwrap();
        write(&f.nonce(&f.data("first")), b"NEW");
    }
    let store = f.store(&grant);
    assert_eq!(store.selected().unwrap().unwrap().record.generation, 1);
    assert_eq!(host::read(f.nonce(&f.data("first"))).unwrap(), b"NEW");
    assert!(matches!(
        store.capture("stale", 0, &mut Faults::default()),
        Err(Error::Conflict)
    ));
    assert!(!f.custody.join("stage-stale").exists());
}
#[test]
fn source_mutation_after_pinning_refuses_before_backup() {
    let f = fresh();
    let grant = f.grant();
    write(&f.source.join("upper.ext4"), b"changed-after-proof");
    let managed = fs::Dir::open(&f.managed, true).unwrap();
    assert!(matches!(
        Store::open(
            &grant,
            fs::Dir::open(&f.custody, true).unwrap(),
            fs::Dir::open(&f.backup, true).unwrap(),
            &[&managed]
        ),
        Err(Error::Custody)
    ));
    assert_eq!(host::read_dir(&f.backup).unwrap().count(), 0);
}
#[test]
fn backup_overlap_and_empty_protected_inventory_refuse() {
    let f = fresh();
    let grant = f.grant();
    let child = f.managed.join("backup");
    private(&child);
    let managed = fs::Dir::open(&f.managed, true).unwrap();
    assert!(matches!(
        Store::open(
            &grant,
            fs::Dir::open(&f.custody, true).unwrap(),
            fs::Dir::open(&child, true).unwrap(),
            &[&managed]
        ),
        Err(Error::Custody)
    ));
    assert!(matches!(
        Store::open(
            &grant,
            fs::Dir::open(&f.custody, true).unwrap(),
            fs::Dir::open(&f.backup, true).unwrap(),
            &[]
        ),
        Err(Error::Custody)
    ));
}
#[test]
fn separate_open_handles_contend_for_custody_lock() {
    let f = fresh();
    let grant = f.grant();
    let _first = f.store(&grant);
    let managed = fs::Dir::open(&f.managed, true).unwrap();
    assert!(
        Store::open(
            &grant,
            fs::Dir::open(&f.custody, true).unwrap(),
            fs::Dir::open(&f.backup, true).unwrap(),
            &[&managed]
        )
        .is_err()
    );
}
#[test]
fn symlinked_custody_and_replaced_parent_are_rejected() {
    let f = fresh();
    let alias = f._temp.path().join("alias");
    symlink(&f.custody, &alias).unwrap();
    assert!(fs::Dir::open(&alias, true).is_err());
    let grant = f.grant();
    let store = f.store(&grant);
    host::rename(&f.custody, f._temp.path().join("moved")).unwrap();
    private(&f.custody);
    assert!(matches!(
        store.capture("first", 0, &mut Faults::default()),
        Err(Error::Custody)
    ));
    assert_eq!(host::read_dir(&f.backup).unwrap().count(), 0);
}
#[test]
fn unsafe_permissions_fail_before_capture() {
    let f = fresh();
    host::set_permissions(&f.custody, host::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        fs::Dir::open(&f.custody, true),
        Err(Error::Custody)
    ));
}
#[test]
fn every_precommit_failure_retains_original_and_does_not_select_staging() {
    for (step, errno) in [
        (Step::Copy, libc::ENOSPC),
        (Step::Metadata, libc::EACCES),
        (Step::FileSync, libc::EIO),
        (Step::DirectorySync, libc::EINVAL),
        (Step::BackupRename, libc::EIO),
        (Step::GenerationRename, libc::EIO),
        (Step::SelectorWrite, libc::ENOSPC),
        (Step::SelectorRename, libc::EACCES),
    ] {
        let f = fresh();
        let grant = f.grant();
        let store = f.store(&grant);
        let mut fault = Faults {
            fail: Some((step, errno)),
        };
        assert!(store.capture("failed", 0, &mut fault).is_err(), "{step:?}");
        assert!(!f.custody.join("active.json").exists());
        assert!(matches!(
            store.selected(),
            Ok(None) | Err(Error::Interrupted)
        ));
        assert!(
            host::read(f.source.join("upper.ext4"))
                .unwrap()
                .starts_with(b"OLD-fsynced-nonce")
        );
        // A name collision/retry never deletes a partial backup/stage.
        if f.custody.join("stage-failed").exists() {
            assert!(store.capture("failed", 0, &mut Faults::default()).is_err());
            assert!(f.custody.join("stage-failed").exists());
        }
    }
}
#[test]
fn uncertain_selector_commit_recovers_without_overwriting_new_writes() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    let mut faults = Faults {
        fail: Some((Step::SelectorSync, libc::EIO)),
    };
    assert!(matches!(
        store.capture("first", 0, &mut faults),
        Err(Error::Uncertain)
    ));
    write(&f.nonce(&f.data("first")), b"NEW-after-uncertain");
    assert_eq!(
        store.recover("first", 0, &mut Faults::default()).unwrap(),
        Outcome::AlreadyCommitted(1)
    );
    assert_eq!(
        host::read(f.nonce(&f.data("first"))).unwrap(),
        b"NEW-after-uncertain"
    );
}
#[test]
fn published_candidate_before_selector_can_be_recovered_with_exact_backup() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    let mut faults = Faults {
        fail: Some((Step::SelectorRename, libc::EACCES)),
    };
    assert!(store.capture("first", 0, &mut faults).is_err());
    assert!(matches!(
        store.selected(),
        Ok(None) | Err(Error::Interrupted)
    ));
    assert_eq!(
        store.recover("first", 0, &mut Faults::default()).unwrap(),
        Outcome::Committed(1)
    );
}
#[test]
fn incomplete_staging_is_not_guessed_or_deleted() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    private(&f.custody.join("stage-first"));
    write(&f.custody.join("stage-first/partial"), b"retain");
    assert!(matches!(
        store.recover("first", 0, &mut Faults::default()),
        Err(Error::Interrupted)
    ));
    assert!(matches!(
        store.capture("first", 0, &mut Faults::default()),
        Err(Error::Collision)
    ));
    assert_eq!(
        host::read(f.custody.join("stage-first/partial")).unwrap(),
        b"retain"
    );
}
#[test]
fn corrupt_unknown_or_symlinked_selector_is_never_treated_as_fresh() {
    for raw in [
        b"".as_slice(),
        b"{}",
        b"[]",
        b"{\"version\":999}",
        b"{\"version\":1,\"version\":1}",
    ] {
        let f = fresh();
        let grant = f.grant();
        let store = f.store(&grant);
        write(&f.custody.join("active.json"), raw);
        assert!(store.capture("first", 0, &mut Faults::default()).is_err());
        assert!(!f.backup.join("first").exists());
    }
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    symlink(f.source.join("upper.ext4"), f.custody.join("active.json")).unwrap();
    assert!(store.selected().is_err());
}
#[test]
fn invalid_receipt_mode_and_foreign_binding_fail_closed() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    let receipt = f.custody.join("generation-first/receipt.json");
    host::set_permissions(&receipt, host::Permissions::from_mode(0o644)).unwrap();
    assert!(store.selected().is_err());
    host::set_permissions(&receipt, host::Permissions::from_mode(0o600)).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&host::read(&receipt).unwrap()).unwrap();
    value["binding"] = serde_json::json!("0".repeat(64));
    write(&receipt, &serde_json::to_vec(&value).unwrap());
    assert!(store.selected().is_err());
}
#[test]
fn committed_missing_root_never_creates_fresh_data() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    host::rename(f.data("first"), f.custody.join("retained-data")).unwrap();
    assert!(store.selected().is_err());
    assert!(!f.data("first").exists());
}
#[test]
fn corrupted_offline_backup_blocks_uncommitted_recovery() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    let mut faults = Faults {
        fail: Some((Step::SelectorRename, libc::EIO)),
    };
    assert!(store.capture("first", 0, &mut faults).is_err());
    write(&f.backup.join("first/data/upper.ext4"), b"tampered");
    assert!(store.recover("first", 0, &mut Faults::default()).is_err());
    assert!(matches!(
        store.selected(),
        Ok(None) | Err(Error::Interrupted)
    ));
}
#[test]
fn rollback_never_overwrites_existing_destination() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    let out = f._temp.path().join("restore");
    private(&out);
    write(&out.join("new"), b"do-not-overwrite");
    assert!(matches!(
        store.rollback_copy(
            "first",
            &fs::Dir::open(&out, true).unwrap(),
            &mut Faults::default()
        ),
        Err(Error::Collision)
    ));
    assert_eq!(host::read(out.join("new")).unwrap(), b"do-not-overwrite");
}
#[test]
fn nested_environment_array_unknown_fields_and_duplicate_fields_are_corrupt() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    let bytes = host::read(f.custody.join("generation-first/receipt.json")).unwrap();
    let record: Record = decode(&bytes).unwrap();
    assert_eq!(record.environment, f.env);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let fields = [
        "source_version",
        "layout",
        "provision",
        "adapter_sha256",
        "engine_sha256",
        "kernel_sha256",
        "init_sha256",
        "profile_sha256",
        "app_sha256",
        "sdk_lock_sha256",
        "provisioning_sha256",
        "lower_sha256",
    ];
    let mut positional = value.clone();
    positional["environment"] = serde_json::Value::Array(
        fields
            .iter()
            .map(|key| value["environment"][*key].clone())
            .collect(),
    );
    assert!(decode::<Record>(&serde_json::to_vec(&positional).unwrap()).is_err());
    let mut unknown = value.clone();
    unknown["environment"]["trusted"] = serde_json::json!(true);
    assert!(decode::<Record>(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut raw = String::from_utf8(bytes).unwrap();
    raw.pop();
    raw.push_str(",\"version\":1}");
    assert!(decode::<Record>(raw.as_bytes()).is_err());
}

#[test]
fn active_restart_and_rebuild_do_not_reimport_changed_original() {
    let f = fresh();
    let grant = f.grant();
    {
        let store = f.store(&grant);
        store.capture("first", 0, &mut Faults::default()).unwrap();
    }
    write(&f.nonce(&f.data("first")), b"CURRENT-NEW");
    write(
        &f.source.join("upper.ext4"),
        b"OLD-source-changed-not-authoritative",
    );
    let store = f.store(&grant);
    assert_eq!(store.selected().unwrap().unwrap().record.generation, 1);
    store
        .rebuild("second", 1, &f.env, &mut Faults::default())
        .unwrap();
    assert_eq!(
        host::read(f.nonce(&f.data("second"))).unwrap(),
        b"CURRENT-NEW"
    );
}

#[test]
fn backup_survives_generation_publish_failure_and_orphan_is_not_replaced() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    let mut fault = Faults {
        fail: Some((Step::GenerationRename, libc::EACCES)),
    };
    assert!(store.capture("failed", 0, &mut fault).is_err());
    assert!(f.backup.join("failed/backup.json").exists());
    assert!(
        host::read(f.backup.join("failed/data/upper.ext4"))
            .unwrap()
            .starts_with(b"OLD-fsynced-nonce")
    );
    assert!(matches!(
        store.recover("failed", 0, &mut Faults::default()),
        Err(Error::Interrupted)
    ));
    // Orphaned staging is NOT an empty installation. A different operation
    // cannot silently recapture or select over it; the only copy is retained.
    assert!(matches!(
        store.capture("fresh", 0, &mut Faults::default()),
        Err(Error::Interrupted)
    ));
    assert!(!f.custody.join("active.json").exists());
    assert!(f.backup.join("failed/backup.json").exists());
}

#[test]
fn copy_preserves_posix_acl_xattr_when_filesystem_accepts_it() {
    let f = Fixture::new(Layout::LegacyClone, Provision::Path, "0.3.1");
    let path = f.source.join("root/workspace/nonce");
    let file = File::open(&path).unwrap();
    // Linux POSIX ACL v2: owner rw, named current-user r, group r, mask r, other0.
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, perm, id) in [
        (1u16, 6u16, u32::MAX),
        (2, 4, fs::uid()),
        (4, 4, u32::MAX),
        (16, 4, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        acl.extend(tag.to_le_bytes());
        acl.extend(perm.to_le_bytes());
        acl.extend(id.to_le_bytes());
    }
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                c"system.posix_acl_access".as_ptr(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        },
        0,
        "test requires ACL-capable local filesystem; unsupported is not PASS"
    );
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("acl", 0, &mut Faults::default()).unwrap();
    let copied = File::open(f.data("acl").join("root/workspace/nonce")).unwrap();
    let mut value = vec![0u8; acl.len()];
    assert_eq!(
        unsafe {
            libc::fgetxattr(
                copied.as_raw_fd(),
                c"system.posix_acl_access".as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
            )
        },
        acl.len() as isize
    );
    assert_eq!(value, acl);
}

#[test]
fn existing_backup_and_generation_names_are_not_replaced() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    private(&f.backup.join("first"));
    write(&f.backup.join("first/keep"), b"only-copy");
    assert!(matches!(
        store.capture("first", 0, &mut Faults::default()),
        Err(Error::Collision)
    ));
    assert_eq!(
        host::read(f.backup.join("first/keep")).unwrap(),
        b"only-copy"
    );
    assert!(!f.custody.join("active.json").exists());
}

#[test]
fn lost_selector_with_retained_generation_refuses_fresh_capture() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    host::rename(
        f.custody.join("active.json"),
        f._temp.path().join("retained-active.json"),
    )
    .unwrap();
    assert!(matches!(store.selected(), Err(Error::Interrupted)));
    assert!(matches!(
        store.capture("fresh", 0, &mut Faults::default()),
        Err(Error::Interrupted)
    ));
    assert!(!f.custody.join("stage-fresh").exists());
    assert!(f.data("first").join("upper.ext4").exists());
}

#[test]
fn stale_rebuild_cas_has_no_backup_or_generation_effects() {
    let f = fresh();
    let grant = f.grant();
    let store = f.store(&grant);
    store.capture("first", 0, &mut Faults::default()).unwrap();
    assert!(matches!(
        store.rebuild("stale", 0, &f.env, &mut Faults::default()),
        Err(Error::Conflict)
    ));
    assert!(!f.custody.join("stage-stale").exists());
    assert!(!f.backup.join("stale").exists());
    assert_eq!(store.selected().unwrap().unwrap().record.generation, 1);
}
