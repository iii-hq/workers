// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (G1). Relaunch after destroy on the SAME operation through
//! the callers' exact sequence: owned Create of the VM directory when absent,
//! ControlFiles of only the MISSING members, `open_log`, `reset_instance`,
//! append, `finish`. Destroy is a committed same-operation Remove issued by a
//! real core receipt. No forged journal, pin or record on positive paths.
use super::*;
use crate::worker_admission::{
    AdmissionContext, MutationPlan, NeverSpawned, PathRequest, RecoveryAction, RecoveryRequest,
    WriterRetirement, testing, with_paths,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
const CAP: u64 = 10 * 1024 * 1024;
struct Pair {
    current: PathBuf,
    previous: PathBuf,
}
impl Pair {
    fn spec(&self) -> LogSpec<'_> {
        LogSpec {
            current: &self.current,
            previous: Some(&self.previous),
            cap_bytes: CAP,
        }
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    context: AdmissionContext,
    lease: Lease,
    data: PathBuf,
    vm: PathBuf,
    console: Pair,
    stderr: Pair,
}
fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
/// The WorkerVms console pair and the sandbox stderr pair, as the callers name them.
fn pairs(vm: &Path) -> (Pair, Pair) {
    (
        Pair {
            current: vm.join("console.log"),
            previous: vm.join("console.log.1"),
        },
        Pair {
            current: vm.join("vm-boot.stderr.log"),
            previous: vm.join("vm-boot.stderr.log.1"),
        },
    )
}
fn declare(lease: Lease, console: &Pair, stderr: &Pair) -> Lease {
    let paths = [
        &console.current,
        &console.previous,
        &stderr.current,
        &stderr.previous,
    ];
    let requests: Vec<_> = paths
        .iter()
        .map(|path| PathRequest {
            path,
            role: PathRole::MutableFile,
        })
        .collect();
    with_paths(lease, &requests).unwrap()
}
fn fixture() -> Fixture {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    let data = temp.path().join("data");
    private(&ns);
    private(&data);
    let context = testing::provision_context(&ns).unwrap();
    let vm = data.join("vm");
    let (console, stderr) = pairs(&vm);
    let lease = declare(context.local(&vm).unwrap(), &console, &stderr);
    Fixture {
        _temp: temp,
        context,
        lease,
        data,
        vm,
        console,
        stderr,
    }
}
/// Owned Create of the VM directory when it is absent.
fn ensure_dir(lease: &Lease, vm: &Path) {
    if !vm.exists() {
        let mut create = lease
            .begin(MutationPlan::Create {
                root: vm.to_path_buf(),
            })
            .unwrap();
        create.create_dir_all(vm, 0o700).unwrap();
        create.commit().unwrap();
    }
}
/// The callers' `open_console`/`initialize_stderr`: one ControlFiles journal of
/// only the MISSING members; an existing member is never replaced here.
fn create_missing(lease: &Lease, pair: &Pair) {
    let missing: Vec<PathBuf> = [&pair.current, &pair.previous]
        .into_iter()
        .filter(|path| !path.exists())
        .cloned()
        .collect();
    if missing.is_empty() {
        return;
    }
    let mut init = lease
        .begin(MutationPlan::ControlFiles {
            files: missing.clone(),
        })
        .unwrap();
    for path in &missing {
        let stage = init.stage_file(path, 0o600).unwrap();
        init.publish_file(stage).unwrap();
    }
    init.commit().unwrap();
}
/// One launch: create what is missing, open, reset, append, finish.
fn launch(lease: &Lease, vm: &Path, pair: &Pair, line: &[u8]) {
    ensure_dir(lease, vm);
    create_missing(lease, pair);
    let writer = lease.open_log(pair.spec()).unwrap();
    writer.reset_instance().unwrap();
    writer.append(line).unwrap();
    writer.finish().unwrap();
}
fn record_in(context: &AdmissionContext, operation: &str, pair: &Pair) -> LogRecord {
    let tx = context.namespace.transaction().unwrap();
    let current = HostPath::new(&pair.current).unwrap();
    tx.operation(operation)
        .unwrap()
        .logs
        .iter()
        .find(|r| r.current == current)
        .expect("ledger slot")
        .clone()
}
fn record(f: &Fixture, pair: &Pair) -> LogRecord {
    record_in(&f.context, &f.lease.0.id, pair)
}
fn ledger_len(f: &Fixture) -> usize {
    let tx = f.context.namespace.transaction().unwrap();
    tx.operation(&f.lease.0.id).unwrap().logs.len()
}
/// The id of the latest committed journal with `plan` (the exact proof parts).
fn latest(f: &Fixture, plan: &str) -> String {
    let tx = f.context.namespace.transaction().unwrap();
    tx.operation(&f.lease.0.id)
        .unwrap()
        .journals
        .iter()
        .rev()
        .find(|j| j.committed && j.plan == plan)
        .expect("committed journal")
        .id
        .clone()
}
fn never_spawned(lease: &Lease) -> NeverSpawned {
    lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap()
}
/// Real spawn and owned `wait`; the receipt is issued by the core.
async fn retired(lease: &Lease) -> WriterRetirement {
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg("exit 0");
    let child = lease.prepare_runtime(&[]).unwrap().spawn(command).unwrap();
    child.activate_own_context_for_test().unwrap();
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    completion.retirement().unwrap().clone()
}
/// Destroy with prune: the whole VM tree, as `remove_runtime_state_admitted`.
fn destroy_prune(lease: &Lease, vm: &Path, receipt: &NeverSpawned) {
    let mut removal = receipt.begin_remove(lease, vm, &[vm]).unwrap();
    removal.remove_path(vm).unwrap();
    removal.commit().unwrap();
}
/// Destroy without prune: exactly the pair, the directory stays.
fn destroy_pair(lease: &Lease, vm: &Path, pair: &Pair, receipt: &NeverSpawned) {
    let mut removal = receipt
        .begin_remove(
            lease,
            vm,
            &[pair.current.as_path(), pair.previous.as_path()],
        )
        .unwrap();
    removal.remove_path(&pair.current).unwrap();
    removal.remove_path(&pair.previous).unwrap();
    removal.commit().unwrap();
}
fn fresh() -> LogCursor {
    LogCursor {
        base: 0,
        len: 0,
        previous_base: 0,
        previous_len: 0,
    }
}
fn identity(path: &Path) -> Identity {
    Identity::of(&fs::File::open(path).unwrap()).unwrap()
}
/// Independent recomputation of the explicit recreation step.
fn expected_chain(before: &LogRecord, removed: &str, created: &str) -> String {
    hex::encode(Sha256::digest(
        format!(
            "{}:recreated:{}:{}:{}:{}:{}",
            before.chain,
            before.sequence,
            removed,
            created,
            before.current_hash,
            before.previous_hash.as_deref().unwrap_or("-"),
        )
        .as_bytes(),
    ))
}
/// The same ledger slot, rebound to the owned new pair with a fresh cursor; the
/// finished history is retained exactly once (step + extended chain).
fn assert_reissued(
    before: &LogRecord,
    after: &LogRecord,
    removed: &str,
    created: &str,
    pair: &Pair,
) {
    assert_eq!(after.id, before.id);
    assert_eq!(after.phase, LogPhase::Clean);
    assert_eq!(after.sequence, before.sequence + 1);
    assert_eq!(after.chain, expected_chain(before, removed, created));
    assert_eq!(
        after.recreated,
        Some(LogRecreation {
            removed: removed.into(),
            created: created.into(),
            previous_chain: before.chain.clone(),
            previous_sequence: before.sequence,
            previous_current_hash: before.current_hash.clone(),
            previous_previous_hash: before.previous_hash.clone(),
        })
    );
    assert_eq!(after.current_id, identity(&pair.current));
    assert_eq!(after.previous_id, Some(identity(&pair.previous)));
    assert_eq!(after.cursor, fresh());
    assert_eq!(after.cap, before.cap);
    assert!(after.bound_epoch > before.bound_epoch);
}
fn request(context: &AdmissionContext, operation: &str) -> RecoveryRequest {
    let tx = context.namespace.transaction().unwrap();
    RecoveryRequest {
        operation_id: operation.into(),
        expected_record_sha256: crate::worker_admission::recovery::digest(
            tx.operation(operation).unwrap(),
        )
        .unwrap(),
        action: RecoveryAction::ReconcileVerified,
    }
}
fn recovered(context: &AdmissionContext, operation: &str) -> bool {
    let report = testing::recover(context, request(context, operation)).unwrap();
    serde_json::to_value(report).unwrap()["status"] == "completed"
        && context
            .namespace
            .transaction()
            .unwrap()
            .operation(operation)
            .unwrap()
            .closed
}
fn assert_untouched(before: &LogRecord, now: &LogRecord) {
    assert_eq!(now.phase, LogPhase::Finished);
    assert_eq!(now.id, before.id);
    assert_eq!(now.chain, before.chain);
    assert_eq!(now.sequence, before.sequence);
    assert_eq!(now.current_id, before.current_id);
    assert_eq!(now.previous_id, before.previous_id);
    assert_eq!(now.recreated, before.recreated);
    assert_eq!(now.bound_epoch, before.bound_epoch);
}

#[tokio::test]
async fn g1_console_relaunch_after_real_wait_destroy_reissues_the_same_record() {
    let f = fixture();
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let writer = f.lease.open_log(f.console.spec()).unwrap();
    writer.reset_instance().unwrap();
    writer.append(b"first launch\n").unwrap();
    let receipt = retired(&f.lease).await;
    // Host pump after the owned wait, then the sole writer finishes.
    writer.append(b"after reap\n").unwrap();
    writer.finish().unwrap();
    let before = record(&f, &f.console);
    let mut removal = receipt
        .begin_remove(&f.lease, &f.vm, &[f.vm.as_path()])
        .unwrap();
    removal.remove_path(&f.vm).unwrap();
    removal.commit().unwrap();
    assert!(!f.vm.exists());
    let removed = latest(&f, "remove");
    // Relaunch on the same operation, through the callers' sequence.
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let created = latest(&f, "control-files");
    let writer = f.lease.open_log(f.console.spec()).unwrap();
    assert_reissued(
        &before,
        &record(&f, &f.console),
        &removed,
        &created,
        &f.console,
    );
    assert_eq!(writer.cursor().unwrap(), fresh());
    assert_eq!(ledger_len(&f), 1);
    writer.reset_instance().unwrap();
    writer.append(b"second launch\n").unwrap();
    writer.finish().unwrap();
    assert_eq!(fs::read(&f.console.current).unwrap(), b"second launch\n");
    // A finished re-issued slot reopens normally: no second recreation step.
    let finished = record(&f, &f.console);
    let again = f.lease.open_log(f.console.spec()).unwrap();
    let reopened = record(&f, &f.console);
    assert_eq!(reopened.recreated, finished.recreated);
    assert_eq!(reopened.chain, finished.chain);
    assert_eq!(reopened.bound_epoch, finished.bound_epoch);
    again.finish().unwrap();
}
#[test]
fn g1_console_and_stderr_relaunch_repeat_past_max_logs_without_growing_the_ledger() {
    let f = fixture();
    let mut finished: Vec<LogRecord> = Vec::new();
    let mut sequences = Vec::new();
    for cycle in 0..MAX_LOGS + 2 {
        ensure_dir(&f.lease, &f.vm);
        let mut now = Vec::new();
        for (index, pair) in [&f.console, &f.stderr].into_iter().enumerate() {
            create_missing(&f.lease, pair);
            let created = latest(&f, "control-files");
            let writer = f.lease.open_log(pair.spec()).unwrap();
            assert_eq!(writer.cursor().unwrap(), fresh());
            let opened = record(&f, pair);
            match finished.get(index) {
                None => assert!(opened.recreated.is_none()),
                Some(before) => {
                    assert_reissued(before, &opened, &latest(&f, "remove"), &created, pair)
                }
            }
            writer.reset_instance().unwrap();
            let line = format!("cycle {cycle}\n");
            writer.append(line.as_bytes()).unwrap();
            writer.finish().unwrap();
            assert_eq!(fs::read(&pair.current).unwrap(), line.into_bytes());
            now.push(record(&f, pair));
        }
        // One slot per declared pair, never one per launch.
        assert_eq!(ledger_len(&f), 2);
        sequences.push(now[0].sequence);
        finished = now;
        let receipt = never_spawned(&f.lease);
        destroy_prune(&f.lease, &f.vm, &receipt);
        assert!(!f.vm.exists());
    }
    assert!(sequences.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(ledger_len(&f), 2);
}
#[test]
fn g1_stderr_exact_pair_destroy_without_prune_reissues_with_a_fresh_cursor() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.stderr, b"launch 0\n");
    for cycle in 1..=3 {
        let before = record(&f, &f.stderr);
        let receipt = never_spawned(&f.lease);
        destroy_pair(&f.lease, &f.vm, &f.stderr, &receipt);
        assert!(f.vm.exists());
        assert!(!f.stderr.current.exists() && !f.stderr.previous.exists());
        let removed = latest(&f, "remove");
        create_missing(&f.lease, &f.stderr);
        let created = latest(&f, "control-files");
        let writer = f.lease.open_log(f.stderr.spec()).unwrap();
        assert_reissued(
            &before,
            &record(&f, &f.stderr),
            &removed,
            &created,
            &f.stderr,
        );
        writer.reset_instance().unwrap();
        writer
            .append(format!("launch {cycle}\n").as_bytes())
            .unwrap();
        writer.finish().unwrap();
        assert_eq!(ledger_len(&f), 1);
    }
}
#[test]
fn g1_unfinished_or_failed_record_keeps_refusing_destroy_and_reissue() {
    // Unfinished: the live writer keeps its holds; the receipt cannot remove.
    let f = fixture();
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let writer = f.lease.open_log(f.console.spec()).unwrap();
    writer.append(b"live\n").unwrap();
    let receipt = never_spawned(&f.lease);
    let refused = receipt
        .begin_remove(&f.lease, &f.vm, &[f.vm.as_path()])
        .err()
        .expect("an unfinished log blocks Remove");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert!(f.console.current.exists());
    assert_eq!(record(&f, &f.console).phase, LogPhase::Clean);
    assert!(f.lease.open_log(f.console.spec()).is_err());
    writer.finish().unwrap();
    // Sticky failure: the faulted writer fences the lifecycle as uncertain.
    let f = fixture();
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let writer = f.lease.open_log(f.console.spec()).unwrap();
    let receipt = never_spawned(&f.lease);
    writer.inject_fault(LogFaultPhase::Outcome);
    assert!(writer.append(b"faulted\n").is_err());
    assert_eq!(record(&f, &f.console).phase, LogPhase::Failed);
    let refused = receipt
        .begin_remove(&f.lease, &f.vm, &[f.vm.as_path()])
        .err()
        .expect("an uncertain lifecycle blocks Remove");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert!(f.vm.exists() && f.console.current.exists());
    assert!(f.lease.open_log(f.console.spec()).is_err());
    assert_eq!(record(&f, &f.console).phase, LogPhase::Failed);
}
#[test]
fn g1_foreign_operation_cannot_remove_recreate_or_reissue_this_ledger() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"mine\n");
    let before = record(&f, &f.console);
    let other = f.data.join("other");
    let foreign = f.context.local(&other).unwrap();
    // Another operation cannot even declare this exact pair.
    let refused = with_paths(
        foreign.clone(),
        &[PathRequest {
            path: &f.console.current,
            role: PathRole::MutableFile,
        }],
    )
    .err()
    .expect("foreign declaration of a live pair");
    assert_eq!(refused.kind(), io::ErrorKind::WouldBlock);
    // Its own pathname-identical pair, destroyed and recreated, re-issues ITS
    // record only; this operation's journals and ledger are untouched.
    let (console_b, stderr_b) = pairs(&other);
    let foreign = declare(foreign, &console_b, &stderr_b);
    launch(&foreign, &other, &console_b, b"theirs\n");
    let receipt = never_spawned(&foreign);
    destroy_prune(&foreign, &other, &receipt);
    ensure_dir(&foreign, &other);
    create_missing(&foreign, &console_b);
    let writer = foreign.open_log(console_b.spec()).unwrap();
    assert!(
        record_in(&f.context, &foreign.0.id, &console_b)
            .recreated
            .is_some()
    );
    writer.finish().unwrap();
    assert_untouched(&before, &record(&f, &f.console));
    // This operation still reopens its intact pair normally.
    let writer = f.lease.open_log(f.console.spec()).unwrap();
    assert!(record(&f, &f.console).recreated.is_none());
    writer.finish().unwrap();
}
#[test]
fn g1_external_recreation_without_control_files_is_refused() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    // Pathname-identical recreation outside any owned Create/ControlFiles.
    private(&f.vm);
    for path in [&f.console.current, &f.console.previous] {
        fs::write(path, b"").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let refused = f
        .lease
        .open_log(f.console.spec())
        .err()
        .expect("externally recreated pair");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        refused.to_string(),
        "path appeared outside an owned mutation"
    );
    assert_untouched(&before, &record(&f, &f.console));
    // Not even an owned recreation can start over the alien inodes.
    assert!(
        f.lease
            .begin(MutationPlan::ControlFiles {
                files: vec![f.console.current.clone(), f.console.previous.clone()],
            })
            .is_err()
    );
}
#[test]
fn g1_only_one_member_removed_and_recreated_is_refused() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    let mut removal = receipt
        .begin_remove(&f.lease, &f.vm, &[f.console.current.as_path()])
        .unwrap();
    removal.remove_path(&f.console.current).unwrap();
    removal.commit().unwrap();
    create_missing(&f.lease, &f.console);
    let refused = f
        .lease
        .open_log(f.console.spec())
        .err()
        .expect("half-recreated pair");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    // With inode reuse the new member can carry the old identity; it is then
    // refused by the finished-size check instead. Either way: no re-issue.
    assert!(
        [
            "log history is unresolved or configuration changed",
            "log size changed after writer finish",
        ]
        .contains(&refused.to_string().as_str()),
        "{refused}"
    );
    assert_untouched(&before, &record(&f, &f.console));
    assert_eq!(
        Some(identity(&f.console.previous)),
        before.previous_id,
        "the retained member is never rebound or replaced"
    );
}
#[test]
fn g1_pathname_identical_foreign_file_after_owned_recreation_is_refused() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    fs::rename(&f.console.current, f.vm.join("held")).unwrap();
    fs::write(&f.console.current, b"foreign\n").unwrap();
    fs::set_permissions(&f.console.current, fs::Permissions::from_mode(0o600)).unwrap();
    let refused = f
        .lease
        .open_log(f.console.spec())
        .err()
        .expect("foreign inode at the recreated path");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        refused.to_string(),
        "admitted identity changed outside an owned mutation"
    );
    assert_untouched(&before, &record(&f, &f.console));
    assert_eq!(fs::read(&f.console.current).unwrap(), b"foreign\n");
}
#[test]
fn g1_recreation_proof_requires_exact_remove_then_create_after_binding() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let op = {
        let tx = f.context.namespace.transaction().unwrap();
        tx.operation(&f.lease.0.id).unwrap().clone()
    };
    let members = vec![
        (
            HostPath::new(&f.console.current).unwrap(),
            before.current_id.clone(),
            identity(&f.console.current),
        ),
        (
            HostPath::new(&f.console.previous).unwrap(),
            before.previous_id.clone().unwrap(),
            identity(&f.console.previous),
        ),
    ];
    assert_eq!(
        recreation_proof(&op, &before, &members),
        Some((latest(&f, "remove"), latest(&f, "control-files")))
    );
    // A binding at or after the Remove (e.g. inode reuse rebound later) proves nothing.
    let mut late = before.clone();
    late.bound_epoch = Some(op.epoch);
    assert_eq!(recreation_proof(&op, &late, &members), None);
    // The Remove must have removed exactly the bound identity.
    let mut wrong_old = members.clone();
    wrong_old[0].1.ino ^= 1;
    assert_eq!(recreation_proof(&op, &before, &wrong_old), None);
    // The creation must have produced exactly the identity now opened.
    let mut wrong_new = members.clone();
    wrong_new[1].2.ino ^= 1;
    assert_eq!(recreation_proof(&op, &before, &wrong_new), None);
    // An uncommitted Remove is not a proof.
    let mut uncommitted = op.clone();
    uncommitted
        .journals
        .iter_mut()
        .rev()
        .find(|j| j.plan == "remove")
        .unwrap()
        .committed = false;
    assert_eq!(recreation_proof(&uncommitted, &before, &members), None);
    // Neither is a creation by any plan other than ControlFiles.
    let mut relabelled = op.clone();
    relabelled
        .journals
        .iter_mut()
        .rev()
        .find(|j| j.plan == "control-files")
        .unwrap()
        .plan = "create".into();
    assert_eq!(recreation_proof(&relabelled, &before, &members), None);
}
#[test]
fn g1_crash_after_remove_commit_recovers_to_one_closed_outcome() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    drop(receipt);
    let Fixture {
        _temp,
        context,
        lease,
        console,
        ..
    } = f;
    let operation = lease.0.id.clone();
    drop(lease);
    assert!(recovered(&context, &operation));
    // The finished history stays exactly once, bound to the removed pair.
    assert_untouched(&before, &record_in(&context, &operation, &console));
}
#[test]
fn g1_crash_between_recreation_and_reissue_recovers_without_reissue() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    drop(receipt);
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let Fixture {
        _temp,
        context,
        lease,
        console,
        ..
    } = f;
    let operation = lease.0.id.clone();
    drop(lease);
    assert!(recovered(&context, &operation));
    assert_untouched(&before, &record_in(&context, &operation, &console));
    assert!(console.current.exists() && console.previous.exists());
}
#[test]
fn g1_crash_mid_recreation_is_resolved_by_recovery_never_by_reissue() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    drop(receipt);
    ensure_dir(&f.lease, &f.vm);
    let mut init = f
        .lease
        .begin(MutationPlan::ControlFiles {
            files: vec![f.console.current.clone(), f.console.previous.clone()],
        })
        .unwrap();
    let first = init.stage_file(&f.console.current, 0o600).unwrap();
    let second = init.stage_file(&f.console.previous, 0o600).unwrap();
    init.publish_file(first).unwrap();
    init.fault_publish_at("after-rename");
    assert!(init.publish_file(second).is_err());
    drop(init);
    // The interrupted recreation fences the lifecycle; nothing is re-issued.
    assert!(f.lease.open_log(f.console.spec()).is_err());
    assert_untouched(&before, &record(&f, &f.console));
    let Fixture {
        _temp,
        context,
        lease,
        console,
        ..
    } = f;
    let operation = lease.0.id.clone();
    drop(lease);
    assert!(recovered(&context, &operation));
    assert_untouched(&before, &record_in(&context, &operation, &console));
    assert!(console.current.exists() && console.previous.exists());
}
#[test]
fn g1_crash_after_reissue_transaction_recovers_the_rebound_record_once() {
    let f = fixture();
    launch(&f.lease, &f.vm, &f.console, b"line\n");
    let before = record(&f, &f.console);
    let receipt = never_spawned(&f.lease);
    destroy_prune(&f.lease, &f.vm, &receipt);
    drop(receipt);
    let removed = latest(&f, "remove");
    ensure_dir(&f.lease, &f.vm);
    create_missing(&f.lease, &f.console);
    let created = latest(&f, "control-files");
    let crashed = f
        .lease
        .open_log_fault(f.console.spec(), LogFaultPhase::Reissue)
        .err()
        .expect("injected re-issue boundary");
    assert_eq!(crashed.to_string(), "injected log re-issue boundary");
    let rebound = record(&f, &f.console);
    assert_reissued(&before, &rebound, &removed, &created, &f.console);
    // No second re-issue: the rebound slot still owns its holds.
    assert_eq!(
        f.lease
            .open_log(f.console.spec())
            .err()
            .expect("slot still held")
            .kind(),
        io::ErrorKind::WouldBlock
    );
    let Fixture {
        _temp,
        context,
        lease,
        console,
        ..
    } = f;
    let operation = lease.0.id.clone();
    drop(lease);
    assert!(recovered(&context, &operation));
    let settled = record_in(&context, &operation, &console);
    assert_eq!(settled.phase, LogPhase::Finished);
    assert_eq!(settled.sequence, before.sequence + 1);
    assert_eq!(settled.chain, rebound.chain);
    assert_eq!(settled.recreated, rebound.recreated);
    assert_eq!(settled.current_id, rebound.current_id);
    let tx = context.namespace.transaction().unwrap();
    assert_eq!(tx.operation(&operation).unwrap().logs.len(), 1);
}
