// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN. No caller production fixture is executed by source work.
use super::*;
use crate::worker_admission::{MutationPlan, PathRequest, testing, with_paths};
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};
struct Fixture {
    _temp: tempfile::TempDir,
    context: crate::worker_admission::AdmissionContext,
    lease: Lease,
    root: PathBuf,
    current: PathBuf,
    old: PathBuf,
}
fn fixture() -> Fixture {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    let root = temp.path().join("runtime");
    std::fs::create_dir(&ns).unwrap();
    std::fs::set_permissions(&ns, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let context = testing::provision_context(&ns).unwrap();
    let current = root.join("console.log");
    let old = root.join("console.log.1");
    let lease = with_paths(
        context.local(&root).unwrap(),
        &[
            PathRequest {
                path: &current,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &old,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    let mut init = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![current.clone(), old.clone()],
        })
        .unwrap();
    for path in [&current, &old] {
        let stage = init.stage_file(path, 0o600).unwrap();
        init.publish_file(stage).unwrap();
    }
    init.commit().unwrap();
    Fixture {
        _temp: temp,
        context,
        lease,
        root,
        current,
        old,
    }
}
fn writer(f: &Fixture, cap: u64) -> LogWriter {
    f.lease
        .open_log(LogSpec {
            current: &f.current,
            previous: Some(&f.old),
            cap_bytes: cap,
        })
        .unwrap()
}
#[test]
fn log_512_records_keep_exact_catalog_cardinality_and_stable_inodes() {
    let f = fixture();
    let log = writer(&f, 4096);
    let before = {
        let tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation(&f.lease.0.id).unwrap();
        (
            op.entries.len(),
            op.journals.len(),
            op.holds.len(),
            op.logs.len(),
        )
    };
    let inode = fs::metadata(&f.current).unwrap().ino();
    for n in 0..512 {
        log.append(format!("{n}\n").as_bytes()).unwrap();
    }
    let tx = f.context.namespace.transaction().unwrap();
    let op = tx.operation(&f.lease.0.id).unwrap();
    assert_eq!(
        (
            op.entries.len(),
            op.journals.len(),
            op.holds.len(),
            op.logs.len()
        ),
        before
    );
    assert_eq!(op.logs[0].sequence, 512);
    drop(tx);
    assert_eq!(fs::metadata(&f.current).unwrap().ino(), inode);
    log.finish().unwrap();
}
#[test]
fn log_rotation_cursors_keep_only_declared_previous_window() {
    let f = fixture();
    let log = writer(&f, 8);
    log.append(b"1234").unwrap();
    assert_eq!(
        log.append(b"56789").unwrap(),
        LogCursor {
            base: 9,
            len: 0,
            previous_base: 0,
            previous_len: 9
        }
    );
    log.append(b"abc").unwrap();
    let read = log.read(2, 64).unwrap();
    assert_eq!(read.bytes, b"3456789abc");
    assert_eq!((read.next, read.end, read.oldest), (12, 12, 0));
    log.append(b"defghij").unwrap();
    let read = log.read(0, 64).unwrap();
    assert_eq!(read.bytes, b"abcdefghij");
    assert_eq!(read.oldest, 9);
    assert_eq!(log.cursor().unwrap().end(), 19);
    log.finish().unwrap();
    assert!(!fs::read_dir(&f.root).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".iii-prior-")
    }));
}
#[test]
fn log_after_intent_effect_rotation_and_outcome_faults_are_sticky() {
    for at in ["intent", "append", "rotation", "outcome"] {
        let f = fixture();
        let log = writer(&f, 3);
        log.fault(at);
        assert!(log.append(b"12345").is_err());
        assert!(log.append(b"do-not-retry").is_err());
        assert!(log.finish().is_err());
        let tx = f.context.namespace.transaction().unwrap();
        assert!(tx.operation(&f.lease.0.id).unwrap().uncertain);
        assert!(
            tx.operation(&f.lease.0.id).unwrap().logs[0]
                .pending
                .is_some()
        );
    }
}
#[test]
fn log_concurrent_writers_share_one_guard_and_finish_waits_for_clones() {
    let f = fixture();
    let log = writer(&f, 4096);
    let copy = log.clone();
    assert!(log.finish().is_err());
    let log = copy;
    std::thread::scope(|s| {
        for _ in 0..4 {
            let log = log.clone();
            s.spawn(move || {
                for _ in 0..100 {
                    log.append(b"x").unwrap();
                }
            });
        }
    });
    assert_eq!(log.cursor().unwrap().end(), 400);
    log.finish().unwrap();
}
#[test]
fn log_logged_layout_can_be_removed_after_writer_finish_without_evidence_bypass() {
    let f = fixture();
    let proof = f
        .lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let log = writer(&f, 32);
    for _ in 0..400 {
        log.append(b"stderr-line\n").unwrap();
    }
    assert!(proof.begin_remove(&f.lease, &f.root, &[&f.root]).is_err());
    log.finish().unwrap();
    let proof = f
        .lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let mut remove = proof.begin_remove(&f.lease, &f.root, &[&f.root]).unwrap();
    remove.remove_path(&f.root).unwrap();
    remove.commit().unwrap();
    assert!(!f.root.exists());
}
#[test]
fn log_fd_replacement_refuses_foreign_bytes_and_protected_evidence_stays_protected() {
    let f = fixture();
    let log = writer(&f, 32);
    fs::rename(&f.current, f.root.join("saved")).unwrap();
    fs::write(&f.current, b"FOREIGN").unwrap();
    assert!(log.append(b"no").is_err());
    assert_eq!(fs::read(&f.current).unwrap(), b"FOREIGN");
    let f = fixture();
    let evidence = f.root.join(".iii-prior-owned");
    fs::write(&evidence, b"KEEP").unwrap();
    assert!(
        f.lease
            .open_log(LogSpec {
                current: &evidence,
                previous: None,
                cap_bytes: 8
            })
            .is_err()
    );
    assert_eq!(fs::read(evidence).unwrap(), b"KEEP");
}
#[test]
fn log_recovery_proves_completed_outcome_without_appending_again() {
    let f = fixture();
    let log = writer(&f, 32);
    log.fault("outcome");
    assert!(log.append(b"ONCE").is_err());
    let mut tx = f.context.namespace.transaction().unwrap();
    let op = tx.operation_mut(&f.lease.0.id).unwrap();
    assert!(reconcile(op).unwrap());
    assert_eq!(op.logs[0].phase, LogPhase::Finished);
    assert_eq!(fs::read(&f.current).unwrap(), b"ONCE");
}

#[test]
fn log_reopen_after_finish_reuses_one_record_and_preserves_cursors() {
    let f = fixture();
    for _ in 0..350 {
        let log = writer(&f, 1024);
        log.append(b"x").unwrap();
        log.finish().unwrap();
    }
    let tx = f.context.namespace.transaction().unwrap();
    let op = tx.operation(&f.lease.0.id).unwrap();
    assert_eq!(op.logs.len(), 1);
    assert_eq!(op.logs[0].cursor.end(), 350);
    assert_eq!(op.journals.len(), 2);
    assert!(op.holds.is_empty());
}
#[test]
fn log_drop_with_unfinished_clone_never_reports_writer_retired() {
    let f = fixture();
    let log = writer(&f, 32);
    let writer_clone = log.clone();
    log.append(b"line").unwrap();
    assert!(log.finish().is_err());
    writer_clone.append(b"still-live").unwrap();
    drop(writer_clone);
    assert!(f.lease.revalidate().is_err());
}

#[test]
fn log_new_instance_reset_is_explicit_and_keeps_chain_and_inodes() {
    let f = fixture();
    let log = writer(&f, 32);
    log.append(b"previous-instance").unwrap();
    let ino = fs::metadata(&f.current).unwrap().ino();
    let chain = {
        let tx = f.context.namespace.transaction().unwrap();
        tx.operation(&f.lease.0.id).unwrap().logs[0].chain.clone()
    };
    log.reset_instance().unwrap();
    assert_eq!(log.cursor().unwrap().end(), 0);
    assert_eq!(fs::metadata(&f.current).unwrap().ino(), ino);
    log.append(b"new").unwrap();
    let tx = f.context.namespace.transaction().unwrap();
    assert_ne!(tx.operation(&f.lease.0.id).unwrap().logs[0].chain, chain);
    drop(tx);
    log.finish().unwrap();
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LogChildFrame {
    namespace: PathBuf,
    admission_fd: i32,
}
#[test]
fn log_owned_child_fixture() {
    use std::io::Read;
    use std::os::fd::{FromRawFd, OwnedFd};
    let args: Vec<_> = std::env::args().collect();
    if !args.windows(2).any(|a| {
        a[0] == "--exact" && a[1] == "worker_admission::log::tests::log_owned_child_fixture"
    }) {
        return;
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut lines = input.lines();
    let frame: LogChildFrame = serde_json::from_str(lines.next().unwrap()).unwrap();
    let runtime_fd: i32 = lines.next().unwrap().parse().unwrap();
    let context = testing::open_context(&frame.namespace).unwrap();
    let admission = context
        .import_child(unsafe { OwnedFd::from_raw_fd(frame.admission_fd) })
        .unwrap();
    let runtime = crate::worker_admission::import_runtime_writer(&admission, unsafe {
        OwnedFd::from_raw_fd(runtime_fd)
    })
    .unwrap();
    runtime.checkpoint().unwrap();
    unsafe { libc::_exit(0) };
}
#[tokio::test]
async fn log_final_pump_after_actual_reap_finishes_without_prior_files_blocking_remove() {
    use std::os::fd::AsRawFd;
    let f = fixture();
    let log = writer(&f, 64);
    let inherited = f
        .lease
        .export_child()
        .unwrap()
        .duplicate_for_exec()
        .unwrap();
    let raw = inherited.as_raw_fd();
    let frame = serde_json::to_string(&LogChildFrame {
        namespace: f._temp.path().join("catalog"),
        admission_fd: raw,
    })
    .unwrap();
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg("exec \"$1\" --exact worker_admission::log::tests::log_owned_child_fixture --nocapture <<EOF\n$2\n$4\nEOF").arg("log-test").arg(std::env::current_exe().unwrap()).arg(frame);
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(raw, libc::F_GETFD);
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            syscall(libc::fcntl(raw, libc::F_SETFD, flags & !libc::FD_CLOEXEC))
        });
    }
    let child = f
        .lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(command)
        .unwrap();
    drop(inherited);
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    let receipt = completion.retirement().unwrap();
    for _ in 0..400 {
        log.append(b"late stderr\n").unwrap();
    }
    log.finish().unwrap();
    let mut remove = receipt.begin_remove(&f.lease, &f.root, &[&f.root]).unwrap();
    remove.remove_path(&f.root).unwrap();
    remove.commit().unwrap();
    assert!(!f.root.exists());
}
