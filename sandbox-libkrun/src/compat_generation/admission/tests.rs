// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::*;
use crate::compat_generation::{Environment, Layout, Provision, Record, Selector, encode, fs};
use std::{
    fs as host,
    os::unix::fs::{PermissionsExt, symlink},
};
fn private(p: &Path) {
    host::create_dir(p).unwrap();
    host::set_permissions(p, host::Permissions::from_mode(0o700)).unwrap();
}
fn write(p: &Path, bytes: &[u8]) {
    host::write(p, bytes).unwrap();
    host::set_permissions(p, host::Permissions::from_mode(0o600)).unwrap();
}
fn record(binding: &str) -> Record {
    Record {
        version: 1,
        binding: binding.into(),
        operation: "current".into(),
        generation: 2,
        parent: 1,
        parent_operation: Some("old".into()),
        environment: Environment {
            source_version: "0.3.1".into(),
            layout: Layout::OverlayImage,
            provision: Provision::Path,
            adapter_sha256: "a".repeat(64),
            engine_sha256: "b".repeat(64),
            kernel_sha256: "c".repeat(64),
            init_sha256: "d".repeat(64),
            profile_sha256: "e".repeat(64),
            app_sha256: "f".repeat(64),
            sdk_lock_sha256: "1".repeat(64),
            provisioning_sha256: "2".repeat(64),
            lower_sha256: Some("3".repeat(64)),
        },
        backup: "current".into(),
        backup_digest: "4".repeat(64),
        snapshot_digest: "5".repeat(64),
    }
}
fn fixture() -> (tempfile::TempDir, String) {
    let root = tempfile::tempdir().unwrap();
    host::set_permissions(root.path(), host::Permissions::from_mode(0o700)).unwrap();
    let binding = "9".repeat(64);
    private(&root.path().join("generation-current"));
    private(&root.path().join("generation-current/data"));
    let bytes = encode(&record(&binding)).unwrap();
    write(&root.path().join("generation-current/receipt.json"), &bytes);
    write(
        &root.path().join("active.json"),
        &encode(&Selector {
            version: 1,
            binding: binding.clone(),
            generation: 2,
            operation: "current".into(),
            record_sha256: fs::digest(&bytes),
        })
        .unwrap(),
    );
    (root, binding)
}
#[test]
fn valid_selector_is_only_custody_evidence_never_boot_authority() {
    let (root, binding) = fixture();
    inspect(root.path(), &binding[..32]).unwrap();
    assert!(inspect(root.path(), &"8".repeat(32)).is_err());
}
#[test]
fn corrupt_selector_receipt_unknown_version_and_symlink_refuse() {
    for kind in ["selector", "version", "receipt", "symlink"] {
        let (root, binding) = fixture();
        let path = root.path().join("active.json");
        if kind == "receipt" {
            write(&root.path().join("generation-current/receipt.json"), b"{}");
        } else if kind == "symlink" {
            host::rename(&path, root.path().join("retained")).unwrap();
            symlink("retained", &path).unwrap();
        } else if kind == "version" {
            let mut value: serde_json::Value =
                serde_json::from_slice(&host::read(&path).unwrap()).unwrap();
            value["version"] = serde_json::json!(2);
            write(&path, &serde_json::to_vec(&value).unwrap());
        } else {
            write(&path, b"{bad");
        }
        assert!(inspect(root.path(), &binding[..32]).is_err());
    }
}
#[test]
fn retained_complete_record_without_active_or_missing_data_is_not_fresh() {
    let (root, binding) = fixture();
    host::rename(
        root.path().join("active.json"),
        root.path().join("retained-selector"),
    )
    .unwrap();
    assert!(inspect(root.path(), &binding[..32]).is_err());
    let (root, binding) = fixture();
    host::rename(
        root.path().join("generation-current/data"),
        root.path().join("retained-data"),
    )
    .unwrap();
    assert!(inspect(root.path(), &binding[..32]).is_err());
}
#[test]
fn inspection_respects_existing_generation_transaction_lock() {
    let (root, binding) = fixture();
    let dir = fs::Dir::open(root.path(), true).unwrap();
    let lock = dir.lock().unwrap();
    assert!(inspect(root.path(), &binding[..32]).is_err());
    drop(lock);
    inspect(root.path(), &binding[..32]).unwrap();
}

#[tokio::test]
async fn valid_active_selection_cannot_grant_remote_start_or_prune() {
    use crate::{
        wire::{WorkerDestroyRequest, WorkerSource, WorkerStartRequest},
        worker_vm::WorkerVms,
    };
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    host::set_permissions(root.path(), host::Permissions::from_mode(0o700)).unwrap();
    let base = root.path().join("workers");
    let binding = hex::encode(Sha256::digest("key"));
    let custody = crate::worker_admission::custody(&base, &binding[..32]).unwrap();
    private(custody.parent().unwrap());
    private(&custody);
    private(&custody.join("generation-current"));
    private(&custody.join("generation-current/data"));
    let bytes = encode(&record(&binding)).unwrap();
    write(&custody.join("generation-current/receipt.json"), &bytes);
    write(
        &custody.join("active.json"),
        &encode(&Selector {
            version: 1,
            binding: binding.clone(),
            generation: 2,
            operation: "current".into(),
            record_sha256: fs::digest(&bytes),
        })
        .unwrap(),
    );
    inspect(&custody, &binding[..32]).unwrap();
    let vms = WorkerVms::new(base.clone(), Some("token".into()), vec![root.path().into()]);
    let result = vms
        .start(WorkerStartRequest {
            auth: "token".into(),
            state_key: "key".into(),
            worker_name: "worker".into(),
            source: WorkerSource::Local {
                host_path: root.path().join("missing-source").display().to_string(),
                run_override: None,
            },
            env: Default::default(),
            engine_url: "ws://127.0.0.1:1/ws".into(),
            build: None,
        })
        .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("compatibility custody")
    );
    for prune in [false, true] {
        assert!(
            vms.destroy(WorkerDestroyRequest {
                auth: "token".into(),
                state_key: "key".into(),
                prune
            })
            .is_err()
        );
    }
    assert!(!base.exists());
    assert_eq!(
        host::read(custody.join("generation-current/receipt.json")).unwrap(),
        bytes
    );
}

#[test]
fn private_handoff_selects_current_new_bytes_and_holds_transition_lease() {
    use crate::compat_generation::{Faults, OfflineGrant, tree};
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    host::set_permissions(root.path(), host::Permissions::from_mode(0o700)).unwrap();
    let owner = root.path().join("owner");
    private(&owner);
    let base = owner.join("workers");
    private(&base);
    let binding = hex::encode(Sha256::digest("key"));
    let vm = &binding[..32];
    let custody = crate::worker_admission::custody(&base, vm).unwrap();
    private(custody.parent().unwrap());
    private(&custody);
    let source = root.path().join("source");
    private(&source);
    write(&source.join("lower.erofs"), b"lower");
    write(&source.join("upper.ext4"), b"OLD");
    let backups = root.path().join("backup");
    private(&backups);
    let mut environment = record(&binding).environment;
    environment.lower_sha256 = Some(fs::digest(b"lower"));
    let source = fs::Dir::open(&source, true).unwrap();
    let source_lease = source.lock().unwrap();
    let pinned = tree::Inventory::scan(&source).unwrap();
    // Test-only proof issuer, never compiled as a production API.
    let transition = TransitionAuthority {
        _offline: OfflineGrant {
            binding: binding.clone(),
            environment: environment.clone(),
            source,
            pinned,
            _source_lease: source_lease,
        },
        _coordinator_custody: File::open(&owner).unwrap(),
    };
    let protected = fs::Dir::open(&base, true).unwrap();
    let store = Store::open(
        &transition._offline,
        fs::Dir::open(&custody, true).unwrap(),
        fs::Dir::open(&backups, true).unwrap(),
        &[&protected],
    )
    .unwrap();
    store.capture("first", 0, &mut Faults::default()).unwrap();
    write(&custody.join("generation-first/data/upper.ext4"), b"NEW");
    let proof = BootAuthority {
        binding: binding.clone(),
        environment,
        _owner_lease: File::open(&owner).unwrap(),
        _retirement_custody: File::open(&owner).unwrap(),
        _sanitization_custody: File::open(&owner).unwrap(),
        _profile_custody: File::open(&owner).unwrap(),
    };
    let selected = handoff(&store, &proof, &transition, &base, vm).unwrap();
    assert_eq!(selected.selected.record.generation, 1);
    let current = tree::Inventory::scan(&selected.selected.data).unwrap();
    assert_eq!(
        current.file_digest(b"upper.ext4"),
        Some(fs::digest(b"NEW").as_str())
    );
    assert!(crate::worker_admission::worker(&base, Some("other-key")).is_err());
    drop(selected);
    assert!(crate::worker_admission::worker(&base, Some("other-key")).is_ok());
    // The intended VM remains refused by public admission despite valid data.
    assert!(crate::worker_admission::worker(&base, Some("key")).is_err());
}
