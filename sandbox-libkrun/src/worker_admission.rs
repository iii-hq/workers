// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Persistent cooperative admission. No data-directory locks, migration issuer,
//! implicit bootstrap, environment override, or permission to boot custody.
mod catalog;
mod endpoint;
mod mutation;
mod owner_read;
mod process_control;
mod recovery;
mod retirement;
mod runtime;
pub(crate) use owner_read::{OwnerReadEntry, OwnerReadGuard};
pub(crate) use process_control::{
    RuntimeControl, RuntimeFailure, RuntimeObservation, RuntimeObserver, RuntimePhase,
    RuntimeSignal,
};
pub(crate) use recovery::{
    RecoveryAction, RecoveryReport, RecoveryRequest, inspect_recovery_for_uid, recover_for_uid,
};
pub(crate) use retirement::{NeverSpawned, WriterRetirement};
pub(crate) use runtime::{
    RuntimeActivity, RuntimeChild, RuntimeCompletion, RuntimeLaunch, RuntimePath, RuntimePathKind,
    RuntimeSpawnError, RuntimeWriter, import_runtime_writer,
};
mod storage;
use catalog::{Entry, Hold, HoldKind, Namespace, overlaps};
pub(crate) use catalog::{bootstrap_for_uid, check_installation};
pub(crate) use endpoint::{BoundEndpoint, EndpointGuard, EndpointListener};
pub(crate) use mutation::{DiagnosticLock, EffectReceipt, MutationGuard, StagedFile, StagedTree};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io,
    os::{fd::OwnedFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use storage::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PathRole {
    Strict,
    MutableTree,
    MutableFile,
    Endpoint,
    ManagedCacheRead,
}
pub(crate) struct PathRequest<'a> {
    pub path: &'a Path,
    pub role: PathRole,
}
pub(crate) enum MutationPlan {
    Create {
        root: PathBuf,
    },
    Adopt {
        root: PathBuf,
    },
    Reclone {
        root: PathBuf,
    },
    Cache {
        root: PathBuf,
    },
    Erofs {
        output: PathBuf,
        spool: PathBuf,
    },
    ImagePair {
        directory: PathBuf,
    },
    Remove {
        root: PathBuf,
        targets: Vec<PathBuf>,
    },
    ControlFiles {
        files: Vec<PathBuf>,
    },
}
#[derive(Clone)]
pub(crate) struct AdmissionContext {
    namespace: Arc<Namespace>,
}
impl AdmissionContext {
    pub(crate) fn system() -> io::Result<Self> {
        Ok(Self {
            namespace: Namespace::production()?,
        })
    }
    pub(crate) fn worker(&self, base: &Path, state_key: Option<&str>) -> io::Result<Lease> {
        worker_in(self.namespace.clone(), base, state_key)
    }
    pub(crate) fn local(&self, path: &Path) -> io::Result<Lease> {
        start(
            self.namespace.clone(),
            vec![entry(path, PathRole::MutableTree)?],
            None,
            false,
        )
    }
    pub(crate) fn reject_retained_path(&self, path: &Path) -> io::Result<()> {
        let tx = self.namespace.transaction()?;
        inspect_entries(&[entry(path, PathRole::Strict)?], &tx.catalog.bases, false)
    }
    pub(crate) fn import_child(&self, fd: OwnedFd) -> io::Result<Lease> {
        import_in(self.namespace.clone(), fd)
    }
}
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    pub(crate) fn provision_context(root: &Path) -> io::Result<AdmissionContext> {
        Ok(AdmissionContext {
            namespace: Namespace::test(root)?,
        })
    }
    pub(crate) fn inspect_recovery(context: &AdmissionContext) -> io::Result<RecoveryReport> {
        recovery::inspect_test(context)
    }
    pub(crate) fn recover(
        context: &AdmissionContext,
        request: RecoveryRequest,
    ) -> io::Result<RecoveryReport> {
        recovery::recover_test(context, request)
    }
    pub(crate) fn open_context(root: &Path) -> io::Result<AdmissionContext> {
        Ok(AdmissionContext {
            namespace: Namespace::open_test(root)?,
        })
    }
}
#[derive(Clone)]
pub(crate) struct Lease(Arc<Held>);
struct Held {
    namespace: Arc<Namespace>,
    id: String,
    sentinel: File,
    holder: String,
}
impl Drop for Held {
    fn drop(&mut self) {
        // Failure leaves an open/uncertain persistent record; never infer
        // retirement from descriptor loss after a failed release transaction.
        let _ = self.namespace.release(&self.id, &self.holder);
    }
}
pub(crate) struct WriterLease(Option<Vec<Lease>>);
impl WriterLease {
    pub(crate) fn new(leases: Vec<Lease>) -> Self {
        Self(Some(leases))
    }
    pub(crate) fn reaped(mut self) {
        drop(self.0.take());
    }
}
impl Drop for WriterLease {
    fn drop(&mut self) {
        if let Some(leases) = self.0.take() {
            std::mem::forget(leases);
        }
    }
}
pub(crate) struct ChildAdmission {
    lease: Lease,
}
impl ChildAdmission {
    pub(crate) fn duplicate_for_exec(&self) -> io::Result<OwnedFd> {
        self.lease.revalidate()?;
        duplicate(&self.lease.0.sentinel)
    }
}
pub(crate) fn import_child(fd: OwnedFd) -> io::Result<Lease> {
    AdmissionContext::system()?.import_child(fd)
}
fn import_in(namespace: Arc<Namespace>, fd: OwnedFd) -> io::Result<Lease> {
    let sentinel = File::from(fd);
    let (id, holder) = namespace.import(&sentinel)?;
    let lease = Lease(Arc::new(Held {
        namespace,
        id,
        sentinel,
        holder,
    }));
    lease.revalidate()?;
    Ok(lease)
}
pub(crate) struct UseGuard {
    lease: Lease,
    id: String,
    path: PathBuf,
    snapshot: Entry,
}
impl UseGuard {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn revalidate(&self) -> io::Result<()> {
        self.lease.revalidate()?;
        validate_entry(&self.snapshot)?;
        let tx = self.lease.0.namespace.transaction()?;
        if !tx
            .operation(&self.lease.0.id)?
            .holds
            .iter()
            .any(|h| h.id == self.id && h.kind == HoldKind::Read)
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl Drop for UseGuard {
    fn drop(&mut self) {
        let _ = self.lease.finish_hold(&self.id, false);
    }
}
impl Lease {
    pub(crate) fn revalidate(&self) -> io::Result<()> {
        let tx = self.0.namespace.transaction()?;
        let op = tx.operation(&self.0.id)?;
        if op.closed || op.uncertain || op.sentinel != Identity::of(&self.0.sentinel)? {
            return Err(refused());
        }
        for entry in &op.entries {
            if !op.holds.iter().any(|h| {
                h.kind != HoldKind::Read
                    && h.path
                        .get()
                        .is_ok_and(|p| overlaps(&p, &entry.path.get().unwrap_or_default()))
            }) {
                validate_entry(entry)?;
            }
        }
        inspect_entries(&op.entries, &tx.catalog.bases, op.transition)
    }
    pub(crate) fn export_child(&self) -> io::Result<ChildAdmission> {
        self.revalidate()?;
        Ok(ChildAdmission {
            lease: self.clone(),
        })
    }
    pub(crate) fn use_path(&self, path: &Path) -> io::Result<UseGuard> {
        let path = absolute(path)?;
        if !present(&path)? {
            return Err(invalid("read path does not exist"));
        }
        let snapshot = entry(&path, PathRole::Strict)?;
        let id = self.hold(&path, HoldKind::Read, None)?;
        if let Err(error) = validate_entry(&snapshot) {
            let _ = self.finish_hold(&id, false);
            return Err(error);
        }
        Ok(UseGuard {
            lease: self.clone(),
            id,
            path,
            snapshot,
        })
    }
    pub(crate) fn begin(&self, plan: MutationPlan) -> io::Result<MutationGuard> {
        MutationGuard::begin(self.clone(), plan)
    }
    pub(crate) fn prepare_runtime(&self, paths: &[RuntimePath<'_>]) -> io::Result<RuntimeLaunch> {
        RuntimeLaunch::prepare(self.clone(), paths)
    }
    pub(crate) fn owner_read_cache(&self, root: &Path) -> io::Result<OwnerReadGuard> {
        OwnerReadGuard::begin(self.clone(), root)
    }
    pub(crate) fn endpoint(&self, path: &Path) -> io::Result<EndpointGuard> {
        EndpointGuard::new(self.clone(), path)
    }
    fn hold(&self, path: &Path, kind: HoldKind, required: Option<PathRole>) -> io::Result<String> {
        self.revalidate()?;
        let mut tx = self.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.0.id)?;
        if op.uncertain || op.closed || op.holds.len() >= 256 {
            return Err(refused());
        }
        if op.entries.iter().any(|e| {
            e.role == PathRole::Strict
                && e.path.get().is_ok_and(|p| p == path)
                && kind == HoldKind::Read
        }) {
            // A strict input must exist at use time, not merely be an absent tail.
            if !present(path)? {
                return Err(invalid("strict input is absent"));
            }
        }
        let covered = op.entries.iter().any(|entry| {
            entry.path.get().is_ok_and(|p| {
                (p == path || (entry.role == PathRole::MutableTree && path.starts_with(&p)))
                    && required.is_none_or(|role| {
                        entry.role == role
                            || (role == PathRole::MutableFile
                                && entry.role == PathRole::MutableTree)
                    })
            })
        });
        if !covered {
            return Err(invalid("path role not declared for this operation"));
        }
        if kind != HoldKind::Read
            && op.entries.iter().any(|e| {
                e.role == PathRole::Strict && e.path.get().is_ok_and(|p| overlaps(&p, path))
            })
        {
            return Err(invalid("strict input aliases mutation"));
        }
        for hold in &op.holds {
            if overlaps(&hold.path.get()?, path)
                && (kind != HoldKind::Read || hold.kind != HoldKind::Read)
            {
                return Err(busy());
            }
        }
        let id = new_id();
        op.holds.push(Hold {
            id: id.clone(),
            path: HostPath::new(path)?,
            kind,
            epoch: op.epoch,
        });
        tx.save()?;
        Ok(id)
    }
    fn finish_hold(&self, id: &str, uncertain: bool) -> io::Result<()> {
        let mut tx = self.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.0.id)?;
        if uncertain {
            op.uncertain = true;
        } else {
            op.holds.retain(|h| h.id != id);
        }
        tx.save()
    }
}
pub(crate) fn refused() -> io::Error {
    invalid(
        "compatibility custody, unknown writer or unsafe admission; real local authority and retirement are required",
    )
}
fn present(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
pub(crate) fn custody(base: &Path, id: &str) -> io::Result<PathBuf> {
    if !id_valid(id) {
        return Err(refused());
    }
    let mut folder = std::ffi::OsString::from(".");
    folder.push(base.file_name().ok_or_else(refused)?);
    folder.push("-compat");
    Ok(base.parent().ok_or_else(refused)?.join(folder).join(id))
}
fn pins(path: &Path, role: PathRole) -> io::Result<Vec<(HostPath, Identity)>> {
    let path = absolute(path)?;
    if role == PathRole::ManagedCacheRead {
        #[cfg(target_os = "linux")]
        {
            let mut here = PathBuf::from("/");
            let mut file = Directory::open(&here)?.file;
            let mut result = vec![(HostPath::new(&here)?, Identity::of(&file)?)];
            for component in path.components() {
                if let std::path::Component::Normal(name) = component {
                    here.push(name);
                    file = open_at(&file, name, libc::O_PATH, 0)?;
                    let meta = file.metadata()?;
                    if meta.file_type().is_symlink() || (!meta.is_dir() && here != path) {
                        return Err(invalid("managed cache no-follow ancestor"));
                    }
                    result.push((HostPath::new(&here)?, Identity::metadata(&meta)));
                }
            }
            if !file.metadata()?.is_dir() || file.metadata()?.uid() != uid() {
                return Err(invalid("managed cache root must be owned directory"));
            }
            return Ok(result);
        }
        #[cfg(not(target_os = "linux"))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "managed cache requires O_PATH",
        ));
    }
    let mut here = PathBuf::from("/");
    let mut dir = Directory::open(&here)?.file;
    let mut output = vec![(HostPath::new(&here)?, Identity::of(&dir)?)];
    let components: Vec<_> = path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(n) = c {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    for (i, name) in components.iter().enumerate() {
        here.push(name);
        let last = i + 1 == components.len();
        match open_at(&dir, name, libc::O_RDONLY | libc::O_DIRECTORY, 0) {
            Ok(next) => {
                output.push((HostPath::new(&here)?, Identity::of(&next)?));
                dir = next;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => {
                if !last {
                    return Err(e);
                }
                let meta = std::fs::symlink_metadata(&here)?;
                let id = Identity::metadata(&meta);
                if meta.is_file() && meta.nlink() == 1 {
                    output.push((HostPath::new(&here)?, id));
                } else if role == PathRole::Endpoint && meta.file_type().is_socket() {
                    output.push((HostPath::new(&here)?, id));
                } else {
                    return Err(invalid("symlink, hardlinked or special input"));
                }
            }
        }
    }
    Ok(output)
}
use std::os::unix::fs::FileTypeExt;
fn entry(path: &Path, role: PathRole) -> io::Result<Entry> {
    let path = absolute(path)?;
    if path == Path::new("/") {
        return Err(invalid("whole-host intervals are not supported"));
    }
    let pins = pins(&path, role)?;
    Ok(Entry {
        path: HostPath::new(&path)?,
        role,
        pins,
    })
}
fn validate_entry(entry: &Entry) -> io::Result<()> {
    let now = pins(&entry.path.get()?, entry.role)?;
    for (path, old) in &entry.pins {
        if !now.iter().any(|(p, new)| {
            p == path
                && old.same_inode(new)
                && (old.mode & libc::S_IFMT == libc::S_IFDIR || old.links == new.links)
        }) {
            return Err(invalid(
                "admitted identity changed outside an owned mutation",
            ));
        }
    }
    // Only core receipts may record newly created ancestors or targets, even
    // for declared output roles. A role is not a license to accept an alien inode.
    if now.len() != entry.pins.len() {
        return Err(invalid("path appeared outside an owned mutation"));
    }
    Ok(())
}
fn inspect_vm(base: &Path, id: &str) -> io::Result<()> {
    let address = custody(base, id)?;
    if present(address.parent().ok_or_else(refused)?)? {
        let registry = Directory::open(address.parent().ok_or_else(refused)?)?;
        registry.private(uid())?;
        for name in registry.names()? {
            if !id_valid(&name) {
                return Err(refused());
            }
        }
    }
    if present(&address)? {
        return Err(refused());
    }
    let runtime = base.join(id);
    if present(&runtime)? {
        Directory::open(&runtime)?;
    }
    for marker in ["compat", "compat-mode", "compat-generation"] {
        if present(&runtime.join(marker))? {
            return Err(refused());
        }
    }
    Ok(())
}
fn inspect_entries(entries: &[Entry], bases: &[HostPath], transition: bool) -> io::Result<()> {
    if transition {
        return Ok(());
    }
    for endpoint in entries.iter().filter(|e| e.role == PathRole::Endpoint) {
        let path = endpoint.path.get()?;
        let parent = path.parent().ok_or_else(|| invalid("endpoint parent"))?;
        if entries.iter().any(|e| {
            e.role != PathRole::Endpoint && e.path.get().is_ok_and(|p| overlaps(&p, parent))
        }) {
            return Err(invalid("endpoint parent overlaps same-operation payload"));
        }
    }
    for entry in entries {
        let path = entry.path.get()?;
        for ancestor in path.ancestors() {
            if ancestor
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.') && n.ends_with("-compat"))
            {
                return Err(refused());
            }
        }
        for encoded in bases {
            let base = encoded.get()?;
            if !overlaps(&path, &base) {
                continue;
            }
            if path == base || base.starts_with(&path) {
                let address = custody(&base, &"0".repeat(32))?;
                if present(address.parent().ok_or_else(refused)?)? {
                    return Err(refused());
                }
                if present(&base)? {
                    Directory::open(&base)?;
                    for (index, child) in std::fs::read_dir(&base)?.enumerate() {
                        if index >= 4096 {
                            return Err(limit());
                        }
                        let child = child?;
                        if let Some(id) = child.file_name().to_str().filter(|id| id_valid(id)) {
                            inspect_vm(&base, id)?;
                        }
                    }
                }
            } else if let Some(id) = path
                .strip_prefix(&base)
                .ok()
                .and_then(|p| p.components().next())
                .and_then(|c| c.as_os_str().to_str())
                .filter(|id| id_valid(id))
            {
                inspect_vm(&base, id)?;
            }
        }
    }
    Ok(())
}
fn start(
    namespace: Arc<Namespace>,
    entries: Vec<Entry>,
    base: Option<HostPath>,
    transition: bool,
) -> io::Result<Lease> {
    let (id, sentinel) = namespace.start(entries, base, transition)?;
    Ok(Lease(Arc::new(Held {
        namespace,
        holder: id.clone(),
        id,
        sentinel,
    })))
}
pub(crate) fn worker(base: &Path, state_key: Option<&str>) -> io::Result<Lease> {
    AdmissionContext::system()?.worker(base, state_key)
}
fn worker_in(namespace: Arc<Namespace>, base: &Path, state_key: Option<&str>) -> io::Result<Lease> {
    let base = absolute(base)?;
    let root = state_key.map_or_else(
        || base.clone(),
        |key| base.join(&hex::encode(Sha256::digest(key))[..32]),
    );
    start(
        namespace,
        vec![entry(&root, PathRole::MutableTree)?],
        Some(HostPath::new(&base)?),
        false,
    )
}
pub(crate) fn local(path: &Path) -> io::Result<Lease> {
    AdmissionContext::system()?.local(path)
}
pub(crate) fn with_inputs(admission: Lease, paths: &[&Path]) -> io::Result<Lease> {
    let requests: Vec<_> = paths
        .iter()
        .map(|path| PathRequest {
            path,
            role: PathRole::Strict,
        })
        .collect();
    with_paths(admission, &requests)
}
pub(crate) fn with_paths(lease: Lease, paths: &[PathRequest<'_>]) -> io::Result<Lease> {
    if paths.is_empty() {
        return Ok(lease);
    }
    lease.revalidate()?;
    if paths.len() > catalog::MAX_PATHS {
        return Err(limit());
    }
    let additions: Vec<_> = paths
        .iter()
        .map(|request| entry(request.path, request.role))
        .collect::<io::Result<_>>()?;
    let mut tx = lease.0.namespace.transaction()?;
    let op = tx.operation(&lease.0.id)?;
    if op.closed || op.uncertain {
        return Err(refused());
    }
    let mut entries = op.entries.clone();
    for addition in additions {
        if let Some(old) = entries.iter().find(|e| e.path == addition.path) {
            if old.role != addition.role {
                return Err(invalid("path role cannot be upgraded or changed"));
            }
            continue;
        }
        if op.holds.iter().any(|h| {
            h.path
                .get()
                .is_ok_and(|p| overlaps(&p, &addition.path.get().unwrap_or_default()))
        }) {
            return Err(busy());
        }
        entries.push(addition);
    }
    tx.check_conflicts(Some(&lease.0.id), &entries, op.transition)?;
    inspect_entries(&entries, &tx.catalog.bases, op.transition)?;
    tx.operation_mut(&lease.0.id)?.entries = entries;
    tx.save()?;
    drop(tx);
    Ok(lease)
}
pub(crate) fn reject_retained_path(path: &Path) -> io::Result<()> {
    AdmissionContext::system()?.reject_retained_path(path)
}
#[cfg(target_os = "linux")]
pub(crate) fn transition(
    base: &Path,
    id: &str,
    _proof: &crate::compat_generation::admission::TransitionAuthority,
) -> io::Result<Lease> {
    if !id_valid(id) {
        return Err(refused());
    }
    let base = absolute(base)?;
    start(
        Namespace::production()?,
        vec![
            entry(&base.join(id), PathRole::MutableTree)?,
            entry(&custody(&base, id)?, PathRole::MutableTree)?,
        ],
        Some(HostPath::new(&base)?),
        true,
    )
}
#[cfg(test)]
mod tests;
