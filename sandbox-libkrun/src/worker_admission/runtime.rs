// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! A real owned wait plus independently free runtime OFD proves writer retirement,
//! never guest flush, application consistency or migration authority.
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
use super::{
    Lease, PathRole,
    catalog::{ActivityFailure, ActivityOutcome, ActivityPhase},
    catalog::{Hold, HoldKind, Namespace, RuntimeEntry, RuntimeRecord, RuntimeState, overlaps},
    storage::*,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RuntimePathKind {
    GuestTree,
    UpperImage,
    ActivityStamp,
}
pub(crate) struct RuntimePath<'a> {
    pub path: &'a Path,
    pub kind: RuntimePathKind,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SentinelRecord {
    version: u32,
    id: String,
    operation: String,
    epoch: u64,
    namespace: Identity,
}
pub(crate) struct RuntimeLaunch {
    lease: Lease,
    id: String,
    sentinel: Option<File>,
    spawned: bool,
}
pub(crate) struct RuntimeChild {
    child: Option<tokio::process::Child>,
    lease: Lease,
    id: String,
    sentinel: Option<File>,
    complete: bool,
}
#[derive(Clone)]
pub(crate) struct RuntimeWriter(Arc<WriterHeld>);
struct WriterHeld {
    lease: Lease,
    id: String,
    sentinel: std::mem::ManuallyDrop<File>,
    failed: AtomicBool,
    activity: Mutex<ActivityLatch>,
}
impl Drop for WriterHeld {
    fn drop(&mut self) {
        // A locally terminal session cannot shed its last writer fence just
        // because persisting the failure also failed. Process exit still needs
        // the durable Pending/Failed outcome check in the real owned waiter.
        if !self.failed.load(Ordering::Acquire) {
            unsafe {
                std::mem::ManuallyDrop::drop(&mut self.sentinel);
            }
        }
    }
}
#[derive(Default)]
struct ActivityLatch {
    failure: Option<ActivityFailure>,
    #[cfg(test)]
    fault: Option<ActivityFault>,
}
#[cfg(test)]
#[derive(Clone, Copy)]
enum ActivityFault {
    Pending,
    Syscall,
    Outcome,
    FailurePersistence,
}
#[derive(Clone)]
pub(crate) struct RuntimeActivity(Arc<ActivityHeld>);
struct ActivityHeld {
    writer: RuntimeWriter,
    file: File,
    parent: Directory,
    path: PathBuf,
    identity: Identity,
}
// Coordination files use positional read/write only. The inherited sentinel
// OFD cursor is a bounded sticky failure latch, retained by the real parent FD
// even after child _exit. No disk write or catalog lock is needed to set it.
fn failed_ofd(file: &File) -> io::Result<bool> {
    let offset = unsafe { libc::lseek(file.as_raw_fd(), 0, libc::SEEK_CUR) };
    if offset < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(offset != 0)
}
fn mark_failed_ofd(file: &File) {
    // The private owned regular FD cannot be closed/replaced by the consumer.
    // Failure to inspect this cursor is itself refusal in the owned waiter.
    let _ = unsafe { libc::lseek(file.as_raw_fd(), 1, libc::SEEK_SET) };
}
fn activity_ready(record: &RuntimeRecord) -> bool {
    record.activity.phase == ActivityPhase::Clean && record.activity.failure.is_none()
}
pub(super) fn validate_sentinel(ns: &Namespace, record: &RuntimeRecord) -> io::Result<File> {
    let n = format!("rt-{}", record.id);
    let file = match ns.directory.file(&n, false) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => ns.archive.file(&n, false)?,
        Err(e) => return Err(e),
    };
    if Identity::of(&file)? != record.sentinel {
        return Err(invalid("runtime sentinel replaced"));
    }
    record.sentinel.regular(ns.uid)?;
    if file.metadata()?.len() > 2048 {
        return Err(limit());
    }
    let binding: SentinelRecord = read_json(&file)?;
    if binding.version != 1
        || binding.id != record.id
        || binding.operation != record.operation
        || binding.epoch != record.epoch
        || !binding
            .namespace
            .same_inode(&Identity::of(&ns.directory.file)?)
    {
        return Err(invalid("runtime binding mismatch"));
    }
    Ok(file)
}
fn validate_paths(record: &RuntimeRecord) -> io::Result<()> {
    for p in &record.paths {
        super::validate_entry(&p.entry)?;
        let path = p.entry.path.get()?;
        let meta = std::fs::symlink_metadata(path)?;
        let valid = match p.kind {
            RuntimePathKind::GuestTree => meta.is_dir(),
            _ => meta.is_file() && meta.nlink() == 1,
        };
        if !valid || meta.uid() != record.sentinel.uid {
            return Err(invalid("runtime root type/owner changed"));
        }
    }
    Ok(())
}
impl RuntimeLaunch {
    pub(super) fn prepare(lease: Lease, paths: &[RuntimePath<'_>]) -> io::Result<Self> {
        if paths.len() > 64 {
            return Err(limit());
        }
        lease.revalidate()?;
        let mut entries = Vec::new();
        for p in paths {
            let path = absolute(p.path)?;
            let role = if p.kind == RuntimePathKind::GuestTree {
                PathRole::MutableTree
            } else {
                PathRole::MutableFile
            };
            let entry = super::entry(&path, role)?;
            let missing_activity =
                p.kind == RuntimePathKind::ActivityStamp && !super::present(&path)?;
            if !missing_activity {
                let meta = std::fs::symlink_metadata(&path)?;
                if meta.uid() != uid()
                    || meta.mode() & 0o022 != 0
                    || (p.kind == RuntimePathKind::GuestTree && !meta.is_dir())
                    || (p.kind != RuntimePathKind::GuestTree
                        && (!meta.is_file() || meta.nlink() != 1))
                {
                    return Err(invalid("runtime requires prepared owned root/file"));
                }
            }
            if entries
                .iter()
                .any(|e: &RuntimeEntry| e.entry.path.get().is_ok_and(|q| overlaps(&q, &path)))
            {
                return Err(invalid("runtime footprints overlap"));
            }
            entries.push(RuntimeEntry {
                entry,
                kind: p.kind,
            });
        }
        let mut tx = lease.0.namespace.transaction()?;
        if tx.catalog.runtimes.len() >= 256 {
            return Err(limit());
        }
        let op = tx.operation_mut(&lease.0.id)?;
        if op.closed || op.uncertain || op.holds.len() + entries.len() > 256 {
            return Err(invalid("runtime operation unavailable"));
        }
        for runtime in &entries {
            let path = runtime.entry.path.get()?;
            if !op
                .entries
                .iter()
                .any(|e| e.path == runtime.entry.path && e.role == runtime.entry.role)
            {
                return Err(invalid("runtime root needs exact explicit role"));
            }
            if op.entries.iter().any(|e| {
                matches!(
                    e.role,
                    PathRole::Strict | PathRole::ManagedCacheRead | PathRole::Endpoint
                ) && e.path.get().is_ok_and(|p| overlaps(&p, &path))
            }) || op
                .holds
                .iter()
                .any(|h| h.path.get().is_ok_and(|p| overlaps(&p, &path)))
            {
                return Err(busy());
            }
        }
        let id = new_id();
        op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
        let epoch = op.epoch;
        for path in &entries {
            op.holds.push(Hold {
                id: id.clone(),
                path: path.entry.path.clone(),
                kind: HoldKind::Runtime,
                epoch,
            });
        }
        let sentinel_file = lease
            .0
            .namespace
            .directory
            .file(&format!("rt-{id}"), true)?;
        sentinel(&sentinel_file)?;
        write_json(
            &sentinel_file,
            &SentinelRecord {
                version: 1,
                id: id.clone(),
                operation: lease.0.id.clone(),
                epoch,
                namespace: Identity::of(&lease.0.namespace.directory.file)?,
            },
        )?;
        tx.catalog.runtimes.push(RuntimeRecord {
            id: id.clone(),
            operation: lease.0.id.clone(),
            sentinel: Identity::of(&sentinel_file)?,
            epoch,
            state: RuntimeState::Preparing,
            paths: entries,
            writer_retired: false,
            guest_consistent: false,
            admission_holder: None,
            activity: ActivityOutcome {
                sequence: 0,
                path: None,
                phase: ActivityPhase::Clean,
                failure: None,
            },
        });
        tx.save()?;
        lease.0.namespace.directory.file.sync_all()?;
        drop(tx);
        let launch = Self {
            lease,
            id,
            sentinel: Some(sentinel_file),
            spawned: false,
        };
        let mut tx = launch.lease.0.namespace.transaction()?;
        let record = tx
            .catalog
            .runtimes
            .iter()
            .find(|r| r.id == launch.id)
            .ok_or_else(|| invalid("runtime preparation missing"))?
            .clone();
        let mut updated = record.paths.clone();
        for runtime in &mut updated {
            if runtime.kind != RuntimePathKind::ActivityStamp {
                continue;
            }
            let path = runtime.entry.path.get()?;
            if runtime
                .entry
                .pins
                .iter()
                .any(|(p, _)| p.get().is_ok_and(|p| p == path))
            {
                continue;
            }
            let parent = Directory::open(path.parent().ok_or_else(|| invalid("activity parent"))?)?;
            if parent.file.metadata()?.uid() != uid() || parent.file.metadata()?.mode() & 0o022 != 0
            {
                return Err(invalid("unsafe activity parent"));
            }
            // Preparing record + exact absent identity is durable before EXCL.
            let file = open_at(
                &parent.file,
                path.file_name().ok_or_else(|| invalid("activity name"))?,
                libc::O_RDWR | libc::O_NONBLOCK | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            file.sync_all()?;
            parent.file.sync_all()?;
            runtime
                .entry
                .pins
                .push((HostPath::new(&path)?, Identity::of(&file)?));
            let op = tx.operation_mut(&launch.lease.0.id)?;
            let entry = op
                .entries
                .iter_mut()
                .find(|e| e.path == runtime.entry.path)
                .ok_or_else(|| invalid("activity declaration disappeared"))?;
            entry.pins = runtime.entry.pins.clone();
        }
        let runtime = tx
            .catalog
            .runtimes
            .iter_mut()
            .find(|r| r.id == launch.id)
            .ok_or_else(|| invalid("runtime missing"))?;
        runtime.paths = updated;
        runtime.state = RuntimeState::Prepared;
        tx.save()?;
        drop(tx);
        Ok(launch)
    }
    pub(crate) fn spawn(
        mut self,
        mut command: tokio::process::Command,
    ) -> io::Result<RuntimeChild> {
        self.lease.revalidate()?;
        {
            let tx = self.lease.0.namespace.transaction()?;
            let record = tx
                .catalog
                .runtimes
                .iter()
                .find(|r| r.id == self.id)
                .ok_or_else(|| invalid("runtime missing before spawn"))?;
            if record.state != RuntimeState::Prepared {
                return Err(invalid("runtime is not prepared"));
            }
            validate_paths(record)?;
        }
        let file = self
            .sentinel
            .as_ref()
            .ok_or_else(|| invalid("runtime launch consumed"))?;
        let fd = duplicate(file)?;
        let raw = fd.as_raw_fd();
        if command.as_std().get_args().any(|a| {
            a == "--runtime-write-fd"
                || a.to_str()
                    .is_some_and(|a| a.starts_with("--runtime-write-fd="))
        }) {
            return Err(invalid("runtime fd argument is core-owned"));
        }
        command.arg("--runtime-write-fd").arg(raw.to_string());
        // Only the child copy changes CLOEXEC; the parent's selected descriptor
        // remains CLOEXEC. No process-global flags or guest environment.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(raw, libc::F_GETFD);
                if flags < 0 {
                    return Err(io::Error::last_os_error());
                }
                syscall(libc::fcntl(raw, libc::F_SETFD, flags & !libc::FD_CLOEXEC))
            });
        }
        match command.spawn() {
            Ok(child) => {
                drop(fd);
                self.spawned = true;
                Ok(RuntimeChild {
                    child: Some(child),
                    lease: self.lease.clone(),
                    id: self.id.clone(),
                    sentinel: self.sentinel.take(),
                    complete: false,
                })
            }
            Err(e) => {
                drop(fd);
                // Do not infer no child effects from an OS spawn error. An
                // unactivated explicit cancellation is handled only before spawn.
                if let Ok(mut tx) = self.lease.0.namespace.transaction() {
                    if let Some(r) = tx.catalog.runtimes.iter_mut().find(|r| r.id == self.id) {
                        r.state = RuntimeState::Uncertain;
                        let _ = tx.save();
                    }
                }
                Err(e)
            }
        }
    }
}
impl Drop for RuntimeLaunch {
    fn drop(&mut self) {
        if !self.spawned {
            let _ = (|| -> io::Result<()> {
                let mut tx = self.lease.0.namespace.transaction()?;
                let record = tx
                    .catalog
                    .runtimes
                    .iter_mut()
                    .find(|r| r.id == self.id)
                    .ok_or_else(|| invalid("runtime missing"))?;
                if record.state != RuntimeState::Prepared {
                    return Err(invalid("activated launch cannot be cancelled"));
                }
                record.state = RuntimeState::Cancelled;
                tx.operation_mut(&self.lease.0.id)?
                    .holds
                    .retain(|h| h.id != self.id);
                tx.save()
            })();
        }
    }
}
impl RuntimeChild {
    pub(crate) fn id(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }
    pub(crate) fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.as_mut().and_then(|c| c.stdin.take())
    }
    pub(crate) fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.as_mut().and_then(|c| c.stdout.take())
    }
    pub(crate) fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.as_mut().and_then(|c| c.stderr.take())
    }
    pub(crate) async fn wait(mut self) -> io::Result<std::process::ExitStatus> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| invalid("runtime child consumed"))?
            .wait()
            .await?;
        // No LOCK_UN on the child-shared OFD. Closing our duplicate precedes
        // the independent nonblocking lock test for every descendant writer.
        if failed_ofd(
            self.sentinel
                .as_ref()
                .ok_or_else(|| invalid("runtime sentinel missing before wait"))?,
        )? {
            return Err(invalid(
                "runtime shared description recorded terminal failure",
            ));
        }
        drop(self.sentinel.take());
        let mut tx = self.lease.0.namespace.transaction()?;
        let record = tx
            .catalog
            .runtimes
            .iter()
            .find(|r| r.id == self.id)
            .ok_or_else(|| invalid("runtime record missing"))?
            .clone();
        let free = validate_sentinel(&self.lease.0.namespace, &record)?;
        sentinel(&free)?;
        if record.state != RuntimeState::Active
            || !activity_ready(&record)
            || tx.operation(&self.lease.0.id)?.uncertain
        {
            return Err(invalid("runtime did not activate or checkpoint failed"));
        }
        validate_paths(&record)?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        // Only the explicitly writable GuestTree descendants may have new
        // identities. Root and ancestor snapshots above never get refreshed.
        for e in &mut op.entries {
            if record.paths.iter().any(|r| {
                r.kind == RuntimePathKind::GuestTree
                    && r.entry.path.get().is_ok_and(|root| {
                        e.path
                            .get()
                            .is_ok_and(|p| p != root && p.starts_with(&root))
                    })
            }) {
                if e.role != PathRole::MutableTree && e.role != PathRole::MutableFile {
                    return Err(invalid("readonly descendant entered runtime union"));
                }
                e.pins = super::pins(&e.path.get()?, e.role)?;
            }
        }
        op.holds.retain(|h| h.id != self.id);
        if let Some(holder) = &record.admission_holder {
            op.holder_ids.retain(|id| id != holder);
            op.holders = op.holder_ids.len() as u32;
        }
        let runtime = tx
            .catalog
            .runtimes
            .iter_mut()
            .find(|r| r.id == self.id)
            .ok_or_else(|| invalid("runtime missing"))?;
        runtime.state = RuntimeState::Retired;
        runtime.writer_retired = true;
        runtime.guest_consistent = false;
        tx.save()?;
        self.complete = true;
        Ok(status)
    }
}
impl Drop for RuntimeChild {
    fn drop(&mut self) {
        if !self.complete {
            if let Some(file) = self.sentinel.take() {
                std::mem::forget(file);
            }
            std::mem::forget(self.lease.clone());
            if let Ok(mut tx) = self.lease.0.namespace.transaction() {
                if let Some(r) = tx.catalog.runtimes.iter_mut().find(|r| r.id == self.id) {
                    r.state = RuntimeState::Uncertain;
                    let _ = tx.save();
                }
            }
        }
    }
}
pub(crate) fn import_runtime_writer(admission: &Lease, fd: OwnedFd) -> io::Result<RuntimeWriter> {
    let file = File::from(fd);
    let identity = Identity::of(&file)?;
    identity.regular(uid())?;
    let binding: SentinelRecord = read_json(&file)?;
    if binding.version != 1
        || binding.operation != admission.0.id
        || !binding
            .namespace
            .same_inode(&Identity::of(&admission.0.namespace.directory.file)?)
    {
        return Err(invalid("runtime import context/operation mismatch"));
    }
    let mut tx = admission.0.namespace.transaction()?;
    let record = tx
        .catalog
        .runtimes
        .iter()
        .find(|r| r.id == binding.id)
        .ok_or_else(|| invalid("runtime not registered"))?
        .clone();
    if record.sentinel != identity
        || record.epoch != binding.epoch
        || record.state != RuntimeState::Prepared
        || tx.operation(&admission.0.id)?.uncertain
    {
        return Err(invalid("runtime state mismatch"));
    }
    let independent = validate_sentinel(&admission.0.namespace, &record)?;
    if ofd(&independent, true)? || !ofd(&file, true)? || failed_ofd(&file)? {
        return Err(invalid("runtime requires inherited sentinel OFD"));
    }
    validate_paths(&record)?;
    if admission.0.holder == admission.0.id {
        return Err(invalid(
            "runtime activation requires the imported child admission holder",
        ));
    }
    let activated = tx
        .catalog
        .runtimes
        .iter_mut()
        .find(|r| r.id == binding.id)
        .ok_or_else(|| invalid("runtime missing"))?;
    activated.state = RuntimeState::Active;
    activated.admission_holder = Some(admission.0.holder.clone());
    tx.save()?;
    Ok(RuntimeWriter(Arc::new(WriterHeld {
        lease: admission.clone(),
        id: binding.id,
        sentinel: std::mem::ManuallyDrop::new(file),
        failed: AtomicBool::new(false),
        activity: Mutex::new(ActivityLatch::default()),
    })))
}
impl RuntimeWriter {
    fn activity_lock(&self) -> io::Result<std::sync::MutexGuard<'_, ActivityLatch>> {
        match self.0.activity.lock() {
            Ok(latch) => Ok(latch),
            Err(poison) => {
                let mut latch = poison.into_inner();
                self.fail_activity(&mut latch, ActivityFailure::Poisoned);
                Err(invalid("runtime activity mutex poisoned"))
            }
        }
    }
    fn checkpoint_locked(&self) -> io::Result<()> {
        if self.0.failed.load(Ordering::Acquire) || failed_ofd(&self.0.sentinel)? {
            return Err(invalid("runtime checkpoint is terminal"));
        }
        let result = (|| -> io::Result<()> {
            let tx = self.0.lease.0.namespace.transaction()?;
            let record = tx
                .catalog
                .runtimes
                .iter()
                .find(|r| r.id == self.0.id)
                .ok_or_else(|| invalid("runtime missing"))?;
            if record.state != RuntimeState::Active
                || !activity_ready(record)
                || record.sentinel != Identity::of(&self.0.sentinel)?
                || tx.operation(&self.0.lease.0.id)?.uncertain
            {
                return Err(invalid("runtime no longer active"));
            }
            validate_paths(record)
        })();
        if result.is_err() {
            self.0.failed.store(true, Ordering::Release);
            if let Ok(mut tx) = self.0.lease.0.namespace.transaction() {
                if let Some(r) = tx.catalog.runtimes.iter_mut().find(|r| r.id == self.0.id) {
                    r.state = RuntimeState::Uncertain;
                    let _ = tx.save();
                }
            }
        }
        result
    }
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        let mut latch = self.activity_lock()?;
        if latch.failure.is_some() {
            return Err(invalid("runtime activity failed terminally"));
        }
        let result = self.checkpoint_locked();
        if result.is_err() {
            self.fail_activity(&mut latch, ActivityFailure::Checkpoint);
        }
        result
    }
    fn fail_activity(&self, latch: &mut ActivityLatch, reason: ActivityFailure) {
        let first = *latch.failure.get_or_insert(reason);
        self.0.failed.store(true, Ordering::Release);
        mark_failed_ofd(&self.0.sentinel);
        // When this persistence fails, an already durable Pending slot remains
        // nonclean. The shared in-process latch never retries the effect.
        if let Ok(mut tx) = self.0.lease.0.namespace.transaction() {
            if let Some(r) = tx.catalog.runtimes.iter_mut().find(|r| r.id == self.0.id) {
                r.activity.phase = ActivityPhase::Failed;
                r.activity.failure = Some(first);
                r.state = RuntimeState::Uncertain;
                let _ = tx.save();
            }
        }
    }
    pub(crate) fn activity(&self, path: &Path) -> io::Result<RuntimeActivity> {
        let mut latch = self.activity_lock()?;
        if latch.failure.is_some() {
            return Err(invalid("runtime activity is terminal"));
        }
        let result = (|| -> io::Result<RuntimeActivity> {
            self.checkpoint_locked()?;
            let path = absolute(path)?;
            let tx = self.0.lease.0.namespace.transaction()?;
            let record = tx
                .catalog
                .runtimes
                .iter()
                .find(|r| r.id == self.0.id)
                .ok_or_else(|| invalid("runtime missing"))?;
            let declared = record
                .paths
                .iter()
                .find(|r| {
                    r.kind == RuntimePathKind::ActivityStamp
                        && r.entry.path.get().is_ok_and(|p| p == path)
                })
                .ok_or_else(|| invalid("activity path not registered"))?;
            let parent = Directory::open(path.parent().ok_or_else(|| invalid("activity parent"))?)?;
            let file = open_at(
                &parent.file,
                path.file_name().ok_or_else(|| invalid("activity name"))?,
                libc::O_RDWR | libc::O_NONBLOCK,
                0,
            )?;
            let identity = Identity::of(&file)?;
            if identity.uid != uid()
                || identity.mode & libc::S_IFMT != libc::S_IFREG
                || identity.links != 1
                || !declared
                    .entry
                    .pins
                    .iter()
                    .any(|(p, id)| p.get().is_ok_and(|p| p == path) && id.same_inode(&identity))
            {
                return Err(invalid("activity inode replaced"));
            }
            Ok(RuntimeActivity(Arc::new(ActivityHeld {
                writer: self.clone(),
                file,
                parent,
                path,
                identity,
            })))
        })();
        if result.is_err() {
            self.fail_activity(&mut latch, ActivityFailure::Identity);
        }
        result
    }
}
impl RuntimeActivity {
    fn validate(&self) -> io::Result<()> {
        self.0.parent.revalidate()?;
        let held = Identity::of(&self.0.file)?;
        let named = Identity::metadata(&std::fs::symlink_metadata(&self.0.path)?);
        if held != self.0.identity || named != self.0.identity {
            return Err(invalid("activity inode/type/owner changed"));
        }
        Ok(())
    }
    pub(crate) fn touch_at(&self, when: SystemTime) -> io::Result<()> {
        let writer = &self.0.writer;
        // Shared across ALL activity handles and writer clones. Lock order is
        // activity mutex -> short catalog transaction, never a callback or wait
        // under the catalog mutex. No application callback executes in core.
        let mut latch = writer.activity_lock()?;
        if latch.failure.is_some() || writer.0.failed.load(Ordering::Acquire) {
            return Err(invalid("runtime activity is terminal"));
        }
        let precheck = writer.checkpoint_locked().and_then(|_| self.validate());
        if let Err(error) = precheck {
            writer.fail_activity(&mut latch, ActivityFailure::Checkpoint);
            return Err(error);
        }
        let pending = (|| -> io::Result<u64> {
            let mut tx = writer.0.lease.0.namespace.transaction()?;
            let record = tx
                .catalog
                .runtimes
                .iter_mut()
                .find(|r| r.id == writer.0.id)
                .ok_or_else(|| invalid("runtime missing"))?;
            if record.state != RuntimeState::Active || !activity_ready(record) {
                return Err(invalid("activity already pending or failed"));
            }
            let sequence = record.activity.sequence.checked_add(1).ok_or_else(limit)?;
            record.activity = ActivityOutcome {
                sequence,
                path: Some(HostPath::new(&self.0.path)?),
                phase: ActivityPhase::Pending,
                failure: None,
            };
            tx.save()?;
            Ok(sequence)
        })();
        let sequence = match pending {
            Ok(n) => n,
            Err(e) => {
                writer.fail_activity(&mut latch, ActivityFailure::PendingPersistence);
                return Err(e);
            }
        };
        #[cfg(test)]
        if matches!(latch.fault, Some(ActivityFault::Pending)) {
            writer.fail_activity(&mut latch, ActivityFailure::PendingPersistence);
            return Err(io::Error::other("injected pending boundary"));
        }
        let effect = (|| -> io::Result<()> {
            self.validate()?;
            #[cfg(test)]
            if matches!(
                latch.fault,
                Some(ActivityFault::Syscall) | Some(ActivityFault::FailurePersistence)
            ) {
                // Fault is at the owned write boundary, NOT a checkpoint refusal.
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            self.0.file.set_modified(when)
        })();
        if let Err(error) = effect {
            #[cfg(test)]
            if matches!(latch.fault, Some(ActivityFault::FailurePersistence)) {
                latch.failure = Some(ActivityFailure::Syscall);
                writer.0.failed.store(true, Ordering::Release);
                mark_failed_ofd(&writer.0.sentinel);
                return Err(error); // Durable Pending deliberately remains.
            }
            writer.fail_activity(&mut latch, ActivityFailure::Syscall);
            return Err(error);
        }
        let outcome = (|| -> io::Result<()> {
            #[cfg(test)]
            if matches!(latch.fault, Some(ActivityFault::Outcome)) {
                return Err(io::Error::other("injected outcome persistence"));
            }
            self.validate()?;
            let mut tx = writer.0.lease.0.namespace.transaction()?;
            let record = tx
                .catalog
                .runtimes
                .iter_mut()
                .find(|r| r.id == writer.0.id)
                .ok_or_else(|| invalid("runtime missing"))?;
            if record.state != RuntimeState::Active
                || record.activity.phase != ActivityPhase::Pending
                || record.activity.sequence != sequence
                || record.activity.path != Some(HostPath::new(&self.0.path)?)
            {
                return Err(invalid("activity outcome cannot overwrite changed ledger"));
            }
            record.activity.phase = ActivityPhase::Clean;
            tx.save()
        })();
        if outcome.is_err() {
            writer.fail_activity(&mut latch, ActivityFailure::OutcomePersistence);
        }
        outcome
    }
}
