// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::{PathRole, storage::*};
use serde::{Deserialize, Serialize};
use std::{fs::File, io, os::fd::AsRawFd, path::Path, sync::Arc};
const ROOT: &str = "/var/lib/iii/worker-admission/v1";
pub(super) const MAX_OPS: usize = 256;
pub(super) const MAX_PATHS: usize = 256;
pub(super) const MAX_BASES: usize = 256;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub path: HostPath,
    pub role: PathRole,
    pub pins: Vec<(HostPath, Identity)>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hold {
    pub id: String,
    pub path: HostPath,
    pub kind: HoldKind,
    pub epoch: u64,
    #[serde(default)]
    pub owner: Option<ResourceOwner>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuntimeOwner {
    pub id: String,
    pub epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResourceOwner {
    pub holder: String,
    pub runtime: Option<RuntimeOwner>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum HoldKind {
    Read,
    Mutation,
    Endpoint,
    Runtime,
    OwnerRead,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Change {
    pub path: HostPath,
    pub old: Option<Identity>,
    pub new: Option<Identity>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub id: String,
    pub plan: String,
    pub root: HostPath,
    pub footprint: Vec<HostPath>,
    pub intents: Vec<EffectIntent>,
    pub permissions: Vec<PermissionIntent>,
    pub epoch: u64,
    pub committed: bool,
    pub changes: Vec<Change>,
    pub artifacts: Vec<HostPath>,
    pub disposals: Vec<Disposal>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EffectIntent {
    pub id: String,
    pub kind: EffectKind,
    pub from: Option<HostPath>,
    pub to: HostPath,
    pub from_parent: Option<Identity>,
    pub to_parent: Identity,
    pub old: Option<Identity>,
    pub new: Option<Identity>,
    pub prior: Option<HostPath>,
    pub completed: bool,
    pub cancelled: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum EffectKind {
    Create,
    Stage,
    Publish,
    Adopt,
    Diagnostic,
    Remove,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PermissionIntent {
    pub path: HostPath,
    pub parent: Identity,
    pub original: Identity,
    pub granted_mode: u32,
    pub acl: Option<String>,
    #[serde(default)]
    pub default_acl: Option<String>,
    pub acl_captured: bool,
    #[serde(default)]
    pub acl_before_grant: bool,
    pub restored: bool,
    pub ctime_changed: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disposal {
    pub stage: String,
    pub slot: HostPath,
    pub path: HostPath,
    pub parent: Identity,
    pub file: Identity,
    pub discarded: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Operation {
    pub id: String,
    pub sentinel: Identity,
    pub transition: bool,
    pub holders: u32,
    pub holder_ids: Vec<String>,
    pub closed: bool,
    pub uncertain: bool,
    pub epoch: u64,
    pub entries: Vec<Entry>,
    pub holds: Vec<Hold>,
    pub journals: Vec<Journal>,
    pub recovery: Option<RecoveryIntent>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    version: u32,
    pub bases: Vec<HostPath>,
    pub operations: Vec<Operation>,
    pub endpoints: Vec<EndpointRecord>,
    pub archives: Vec<ArchiveRecord>,
    pub runtimes: Vec<RuntimeRecord>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EndpointRecord {
    pub id: String,
    pub path: HostPath,
    pub identity: Option<Identity>,
    pub operation: String,
    pub epoch: u64,
    pub sentinel: Identity,
    pub parent: Identity,
    pub state: EndpointState,
    #[serde(default)]
    pub hold_id: Option<String>,
    #[serde(default)]
    pub owner: Option<ResourceOwner>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum EndpointState {
    BindIntent,
    Bound,
    CloseIntent,
    Retired,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuntimeRecord {
    pub id: String,
    pub operation: String,
    pub sentinel: Identity,
    pub epoch: u64,
    pub state: RuntimeState,
    pub paths: Vec<RuntimeEntry>,
    pub writer_retired: bool,
    pub guest_consistent: bool,
    pub admission_holder: Option<String>,
    pub activity: ActivityOutcome,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ActivityFailure {
    Checkpoint,
    Identity,
    PendingPersistence,
    Syscall,
    OutcomePersistence,
    Poisoned,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ActivityPhase {
    Clean,
    Pending,
    Failed,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivityOutcome {
    pub sequence: u64,
    pub path: Option<HostPath>,
    pub phase: ActivityPhase,
    pub failure: Option<ActivityFailure>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum RuntimeState {
    Preparing,
    Prepared,
    Active,
    Retired,
    Cancelled,
    Uncertain,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuntimeEntry {
    pub entry: Entry,
    pub kind: super::runtime::RuntimePathKind,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecoveryIntent {
    pub record_sha256: String,
    pub completed: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArchiveRecord {
    pub operation: String,
    pub digest: String,
    pub metadata: Option<Identity>,
    pub sentinel: Identity,
    pub endpoints: Vec<EndpointRecord>,
    pub runtimes: Vec<RuntimeRecord>,
    pub complete: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    uid: u32,
    namespace: Identity,
    mutex: Identity,
    catalog: Identity,
    previous: Identity,
    archive: Identity,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sentinel {
    version: u32,
    id: String,
    namespace: Identity,
}
pub(super) struct Namespace {
    pub directory: Directory,
    pub(super) uid: u32,
    manifest: Identity,
    mutex: Identity,
    catalog: Identity,
    previous: Identity,
    pub(super) archive: Directory,
    administrative: bool,
}
pub(super) struct Transaction<'a> {
    namespace: &'a Namespace,
    _mutex: File,
    file: File,
    previous: File,
    pub catalog: Catalog,
}
impl Namespace {
    pub(super) fn administrative(expected_uid: u32) -> io::Result<Arc<Self>> {
        if uid() != 0 {
            return Err(invalid("recovery requires effective administrative uid0"));
        }
        Self::open(
            &Path::new(ROOT).join(expected_uid.to_string()),
            expected_uid,
            true,
        )
        .map(Arc::new)
    }
    pub(super) fn production() -> io::Result<Arc<Self>> {
        Self::open(Path::new(ROOT).join(uid().to_string()).as_path(), uid(), true).map(Arc::new).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound { io::Error::new(io::ErrorKind::NotFound, "worker admission setup required: explicit administrative bootstrap_for_uid has not provisioned this euid") } else { e }
        })
    }
    fn open(path: &Path, expected_uid: u32, administrative: bool) -> io::Result<Self> {
        if administrative {
            Directory::open(path.parent().ok_or_else(|| invalid("namespace parent"))?)?
                .administrative()?;
        }
        let directory = Directory::open(path)?;
        directory.private(expected_uid)?;
        let mf = directory.file("manifest.json", false)?;
        Identity::of(&mf)?.regular(expected_uid)?;
        if mf.metadata()?.len() > 2048 {
            return Err(limit());
        }
        let manifest: Manifest = read_json(&mf)?;
        if manifest.version != 1
            || manifest.uid != expected_uid
            || !manifest
                .namespace
                .same_inode(&Identity::of(&directory.file)?)
        {
            return Err(invalid("namespace manifest mismatch"));
        }
        for (n, expected) in [
            ("catalog.lock", &manifest.mutex),
            ("catalog.json", &manifest.catalog),
            ("catalog.previous", &manifest.previous),
        ] {
            let file = directory.file(n, false)?;
            let actual = Identity::of(&file)?;
            actual.regular(expected_uid)?;
            if &actual != expected {
                return Err(invalid("coordination inode replaced"));
            }
        }
        let archive = Directory::open(&directory.path.join("archive"))?;
        archive.private(expected_uid)?;
        if !manifest.archive.same_inode(&Identity::of(&archive.file)?) {
            return Err(invalid("archive namespace replaced"));
        }
        let ns = Self {
            directory,
            uid: expected_uid,
            manifest: Identity::of(&mf)?,
            mutex: manifest.mutex,
            catalog: manifest.catalog,
            previous: manifest.previous,
            archive,
            administrative,
        };
        ns.transaction()?;
        Ok(ns)
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        self.directory.revalidate()?;
        self.directory.private(self.uid)?;
        self.archive.revalidate()?;
        self.archive.private(self.uid)?;
        let mf = self.directory.file("manifest.json", false)?;
        if Identity::of(&mf)? != self.manifest {
            return Err(invalid("manifest inode replaced"));
        }
        if mf.metadata()?.len() > 2048 {
            return Err(limit());
        }
        let manifest: Manifest = read_json(&mf)?;
        if manifest.version != 1
            || manifest.uid != self.uid
            || manifest.mutex != self.mutex
            || manifest.catalog != self.catalog
            || manifest.previous != self.previous
            || !manifest
                .archive
                .same_inode(&Identity::of(&self.archive.file)?)
            || !manifest
                .namespace
                .same_inode(&Identity::of(&self.directory.file)?)
        {
            return Err(invalid("manifest content changed"));
        }
        if self.administrative {
            Directory::open(
                self.directory
                    .path
                    .parent()
                    .ok_or_else(|| invalid("namespace parent"))?,
            )?
            .administrative()?;
        }
        Ok(())
    }
    pub(super) fn transaction(&self) -> io::Result<Transaction<'_>> {
        self.validate()?;
        let mutex = self.directory.file("catalog.lock", false)?;
        if Identity::of(&mutex)? != self.mutex {
            return Err(invalid("catalog mutex replaced"));
        }
        flock(&mutex)?;
        let file = self.directory.file("catalog.json", false)?;
        if Identity::of(&file)? != self.catalog {
            return Err(invalid("catalog inode replaced"));
        }
        let previous = self.directory.file("catalog.previous", false)?;
        if Identity::of(&previous)? != self.previous {
            return Err(invalid("previous catalog inode replaced"));
        }
        let _: Catalog = read_json(&previous)?;
        let catalog: Catalog = read_json(&file)?;
        let tx = Transaction {
            namespace: self,
            _mutex: mutex,
            file,
            previous,
            catalog,
        };
        tx.validate()?;
        Ok(tx)
    }
    pub(super) fn start(
        self: &Arc<Self>,
        entries: Vec<Entry>,
        base: Option<HostPath>,
        transition: bool,
    ) -> io::Result<(String, File)> {
        let mut tx = self.transaction()?;
        // Completed evidence is removed only by explicit administrative archival.
        if tx.catalog.operations.len() >= MAX_OPS {
            return Err(limit());
        }
        if let Some(base) = base {
            if !tx.catalog.bases.contains(&base) {
                let new_base = base.get()?;
                for existing in &tx.catalog.operations {
                    if !existing.closed
                        && existing
                            .entries
                            .iter()
                            .any(|e| e.path.get().is_ok_and(|p| overlaps(&p, &new_base)))
                    {
                        return Err(busy());
                    }
                }
                if tx.catalog.bases.len() >= MAX_BASES {
                    return Err(limit());
                }
                tx.catalog.bases.push(base);
            }
        }
        tx.check_conflicts(None, &entries, transition)?;
        super::inspect_entries(&entries, &tx.catalog.bases, transition)?;
        let id = new_id();
        let file = self.directory.file(&format!("op-{id}"), true)?;
        sentinel(&file)?;
        write_json(
            &file,
            &Sentinel {
                version: 1,
                id: id.clone(),
                namespace: Identity::of(&self.directory.file)?,
            },
        )?;
        let identity = Identity::of(&file)?;
        identity.regular(self.uid)?;
        tx.catalog.operations.push(Operation {
            id: id.clone(),
            sentinel: identity,
            transition,
            holders: 1,
            holder_ids: vec![id.clone()],
            closed: false,
            uncertain: false,
            epoch: 0,
            entries,
            holds: Vec::new(),
            journals: Vec::new(),
            recovery: None,
        });
        tx.save()?;
        self.directory.file.sync_all()?;
        Ok((id, file))
    }
    pub(super) fn import(self: &Arc<Self>, file: &File) -> io::Result<(String, String)> {
        let identity = Identity::of(file)?;
        identity.regular(self.uid)?;
        let record: Sentinel = read_json(file)?;
        if record.version != 1
            || !id_valid(&record.id)
            || !record
                .namespace
                .same_inode(&Identity::of(&self.directory.file)?)
        {
            return Err(invalid("invalid child sentinel"));
        }
        let mut tx = self.transaction()?;
        let op = tx.operation_mut(&record.id)?;
        if op.sentinel != identity || op.closed || op.uncertain || op.transition {
            return Err(invalid("child operation mismatch"));
        }
        let independent = self.directory.file(&format!("op-{}", record.id), false)?;
        // The independent description must observe our OFD write lock; the
        // inherited description must see no conflict with its OWN lock.
        if ofd(&independent, true)? || !ofd(file, true)? {
            return Err(invalid("child does not hold the original sentinel OFD"));
        }
        op.holders = op
            .holders
            .checked_add(1)
            .filter(|n| *n <= 256)
            .ok_or_else(limit)?;
        let holder = new_id();
        op.holder_ids.push(holder.clone());
        tx.save()?;
        Ok((record.id, holder))
    }
    pub(super) fn release(&self, id: &str, holder: &str) -> io::Result<()> {
        let mut tx = self.transaction()?;
        let op = tx.operation_mut(id)?;
        let index = op
            .holder_ids
            .iter()
            .position(|id| id == holder)
            .ok_or_else(|| invalid("unknown holder release"))?;
        // A dropped imported lease is not evidence that its outstanding
        // resources ended (_exit may skip every guard). Keep membership until
        // exact resource completion or verified runtime finalization transfers it.
        if op
            .holds
            .iter()
            .any(|h| h.owner.as_ref().is_none_or(|o| o.holder == holder))
        {
            return Ok(());
        }
        op.holder_ids.remove(index);
        op.holders = op.holder_ids.len() as u32;
        if op.holders == 0
            && !op.uncertain
            && op.holds.is_empty()
            && op.journals.iter().all(|j| j.committed)
        {
            op.closed = true;
        }
        tx.save()
    }
    #[cfg(test)]
    pub(super) fn open_test(root: &Path) -> io::Result<Arc<Self>> {
        Self::open(root, uid(), false).map(Arc::new)
    }
    #[cfg(test)]
    pub(super) fn test(root: &Path) -> io::Result<Arc<Self>> {
        let directory = Directory::open(root)?;
        directory.private(uid())?;
        provision(&directory, uid())?;
        Self::open(root, uid(), false).map(Arc::new)
    }
}
impl Transaction<'_> {
    fn validate(&self) -> io::Result<()> {
        if self.catalog.version != 1
            || self.catalog.operations.len() > MAX_OPS
            || self.catalog.bases.len() > MAX_BASES
        {
            return Err(limit());
        }
        if self.catalog.endpoints.len() > 256 {
            return Err(limit());
        }
        if self.catalog.runtimes.len() > 256 {
            return Err(limit());
        }
        let mut runtime_ids = std::collections::HashSet::new();
        for runtime in &self.catalog.runtimes {
            if (runtime.activity.phase == ActivityPhase::Failed)
                != (runtime.activity.failure.is_some())
                || (runtime.activity.phase == ActivityPhase::Pending
                    && runtime.activity.path.is_none())
                || (runtime.state == RuntimeState::Retired
                    && runtime.activity.phase != ActivityPhase::Clean)
            {
                return Err(invalid("invalid activity outcome"));
            }
            if let Some(path) = &runtime.activity.path {
                path.get()?;
                if !runtime.paths.iter().any(|p| {
                    p.kind == super::runtime::RuntimePathKind::ActivityStamp
                        && &p.entry.path == path
                }) {
                    return Err(invalid("activity outcome outside declared footprint"));
                }
            }
            if runtime.writer_retired != (runtime.state == RuntimeState::Retired)
                || runtime
                    .admission_holder
                    .as_ref()
                    .is_some_and(|h| !id_valid(h))
            {
                return Err(invalid("invalid runtime retirement receipt"));
            }
            if !id_valid(&runtime.id)
                || !runtime_ids.insert(&runtime.id)
                || runtime.paths.len() > 64
                || runtime.guest_consistent
                || !self
                    .catalog
                    .operations
                    .iter()
                    .any(|op| op.id == runtime.operation && runtime.epoch <= op.epoch)
            {
                return Err(invalid("invalid runtime record"));
            }
            super::runtime::validate_sentinel(self.namespace, runtime)?;
            for path in &runtime.paths {
                path.entry.path.get()?;
                let role = if path.kind == super::runtime::RuntimePathKind::GuestTree {
                    PathRole::MutableTree
                } else {
                    PathRole::MutableFile
                };
                if path.entry.role != role {
                    return Err(invalid("runtime role mismatch"));
                }
                if path.entry.pins.len() > 128 {
                    return Err(limit());
                }
            }
        }
        let mut endpoint_paths = std::collections::HashSet::new();
        let mut endpoint_ids = std::collections::HashSet::new();
        for endpoint in &self.catalog.endpoints {
            let path = endpoint.path.get()?;
            if !id_valid(&endpoint.id)
                || !endpoint_ids.insert(&endpoint.id)
                || (endpoint.state != EndpointState::Retired && !endpoint_paths.insert(path))
                || endpoint.identity.as_ref().is_some_and(|id| {
                    id.mode & libc::S_IFMT != libc::S_IFSOCK || id.uid != self.namespace.uid
                })
                || !self
                    .catalog
                    .operations
                    .iter()
                    .any(|op| op.id == endpoint.operation && endpoint.epoch <= op.epoch)
            {
                return Err(invalid("invalid endpoint receipt"));
            }
            if endpoint.parent.mode & libc::S_IFMT != libc::S_IFDIR
                || endpoint.parent.uid != self.namespace.uid
                || endpoint.parent.mode & 0o7777 != 0o700
                || (endpoint.state != EndpointState::BindIntent && endpoint.identity.is_none())
            {
                return Err(invalid("invalid endpoint parent/state"));
            }
            let op = self.operation(&endpoint.operation)?;
            match (&endpoint.hold_id, &endpoint.owner) {
                (Some(id), Some(owner)) => {
                    if !id_valid(id) {
                        return Err(invalid("endpoint hold id invalid"));
                    }
                    self.validate_resource_owner(
                        op,
                        owner,
                        endpoint.state == EndpointState::Retired,
                        Some(endpoint),
                    )?;
                    let matching = op.holds.iter().filter(|h| h.id == *id).collect::<Vec<_>>();
                    if endpoint.state == EndpointState::Retired {
                        if !matching.is_empty() {
                            return Err(invalid("retired endpoint still owns a hold"));
                        }
                    } else if matching.len() != 1
                        || matching[0].kind != HoldKind::Endpoint
                        || matching[0].path != endpoint.path
                        || matching[0].owner.as_ref() != Some(owner)
                        || matching[0].epoch > endpoint.epoch
                    {
                        return Err(invalid("endpoint record/hold owner mismatch"));
                    }
                }
                (None, None) => (), // Parsed legacy evidence is never cleanup authority.
                _ => return Err(invalid("partial endpoint ownership")),
            }
            super::endpoint::validate_sentinel(self.namespace, endpoint)?;
        }
        let mut allowed = vec![
            "manifest.json".to_string(),
            "catalog.lock".to_string(),
            "catalog.json".to_string(),
            "catalog.previous".to_string(),
            "archive".to_string(),
        ];
        for endpoint in &self.catalog.endpoints {
            if self
                .namespace
                .directory
                .path
                .join(format!("ep-{}", endpoint.id))
                .try_exists()?
            {
                allowed.push(format!("ep-{}", endpoint.id));
            }
        }
        for runtime in &self.catalog.runtimes {
            let name = format!("rt-{}", runtime.id);
            if self.namespace.directory.path.join(&name).try_exists()? {
                allowed.push(name);
            }
        }
        super::recovery::validate_archives(self.namespace, &self.catalog)?;
        let mut ids = std::collections::HashSet::new();
        let mut bases = std::collections::HashSet::new();
        for base in &self.catalog.bases {
            if !bases.insert(base.get()?) {
                return Err(invalid("duplicate registered base"));
            }
        }
        for op in &self.catalog.operations {
            if !id_valid(&op.id)
                || !ids.insert(&op.id)
                || op.entries.len() > MAX_PATHS
                || op.holds.len() > 256
                || op.journals.len() > 256
                || op.holders > 256
                || op.holder_ids.len() != op.holders as usize
                || op.holder_ids.iter().any(|id| !id_valid(id))
                || op
                    .holder_ids
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != op.holder_ids.len()
            {
                return Err(invalid("invalid catalog operation bounds"));
            }
            if let Some(intent) = &op.recovery {
                if intent.record_sha256.len() != 64
                    || !intent.record_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(invalid("invalid recovery intent"));
                }
            }
            let n = format!("op-{}", op.id);
            if self.namespace.directory.path.join(&n).try_exists()? {
                allowed.push(n.clone());
            }
            let sentinel_file = self.operation_sentinel(op)?;
            if Identity::of(&sentinel_file)? != op.sentinel {
                return Err(invalid("sentinel replaced"));
            }
            op.sentinel.regular(self.namespace.uid)?;
            if sentinel_file.metadata()?.len() > 2048 {
                return Err(limit());
            }
            let sentinel_record: Sentinel = read_json(&sentinel_file)?;
            if sentinel_record.version != 1
                || sentinel_record.id != op.id
                || !sentinel_record
                    .namespace
                    .same_inode(&Identity::of(&self.namespace.directory.file)?)
            {
                return Err(invalid("sentinel binding mismatch"));
            }
            let mut paths = std::collections::HashSet::new();
            let mut total_pins = 0usize;
            for entry in &op.entries {
                let path = entry.path.get()?;
                if !paths.insert(path.clone()) {
                    return Err(invalid("duplicate interval"));
                }
                total_pins += entry.pins.len();
                if total_pins > 8192 {
                    return Err(limit());
                }
                if entry.pins.len() > 128 {
                    return Err(limit());
                }
                let mut seen = std::collections::HashSet::new();
                for (p, identity) in &entry.pins {
                    let p = p.get()?;
                    if !path.starts_with(&p) || !seen.insert(p) || identity.ino == 0 {
                        return Err(invalid("invalid identity pin"));
                    }
                }
            }
            let mut resource_ids = std::collections::HashSet::new();
            for hold in &op.holds {
                hold.path.get()?;
                if op
                    .holds
                    .iter()
                    .any(|other| other.id == hold.id && other.kind != hold.kind)
                {
                    return Err(invalid("hold id reused across resource kinds"));
                }
                if matches!(hold.kind, HoldKind::Read | HoldKind::Endpoint)
                    && !resource_ids.insert(&hold.id)
                {
                    return Err(invalid("duplicate resource hold id"));
                }
                if let Some(owner) = &hold.owner {
                    self.validate_resource_owner(op, owner, false, None)?;
                    if owner.runtime.as_ref().is_some_and(|r| r.epoch > hold.epoch) {
                        return Err(invalid("resource hold predates runtime issuer"));
                    }
                    if owner.runtime.is_some()
                        && !matches!(hold.kind, HoldKind::Read | HoldKind::Endpoint)
                    {
                        return Err(invalid("runtime resource owner on unsupported hold kind"));
                    }
                    if hold.kind == HoldKind::Read
                        && owner.runtime.as_ref().is_some_and(|bound| {
                            self.catalog
                                .runtimes
                                .iter()
                                .any(|r| r.id == bound.id && r.state == RuntimeState::Retired)
                        })
                    {
                        return Err(invalid("retired runtime retains a bound read hold"));
                    }
                }
                if !id_valid(&hold.id) || hold.epoch > op.epoch {
                    return Err(invalid("invalid hold"));
                }
            }
            let mut journals = std::collections::HashSet::new();
            for journal in &op.journals {
                journal.root.get()?;
                if journal.footprint.is_empty()
                    || journal.footprint.len() > 256
                    || journal.intents.len() > 512
                {
                    return Err(limit());
                }
                for path in &journal.footprint {
                    path.get()?;
                }
                let mut intent_ids = std::collections::HashSet::new();
                for intent in &journal.intents {
                    intent.to.get()?;
                    if let Some(p) = &intent.from {
                        p.get()?;
                    }
                    if let Some(p) = &intent.prior {
                        p.get()?;
                    }
                    if !id_valid(&intent.id)
                        || !intent_ids.insert(&intent.id)
                        || (intent.completed && intent.cancelled)
                        || intent.to_parent.mode & libc::S_IFMT != libc::S_IFDIR
                    {
                        return Err(invalid("invalid before-effect intent"));
                    }
                }
                if !journals.insert(&journal.id)
                    || ![
                        "create",
                        "adopt",
                        "reclone",
                        "cache",
                        "erofs",
                        "image-pair",
                        "remove",
                        "owner-read",
                        "control-files",
                    ]
                    .contains(&journal.plan.as_str())
                    || journal.disposals.len() > 256
                {
                    return Err(invalid("invalid mutation plan/id"));
                }
                if journal.plan == "erofs"
                    && (journal.footprint.len() != 2
                        || journal.footprint[0] == journal.footprint[1]
                        || overlaps(&journal.footprint[0].get()?, &journal.footprint[1].get()?))
                {
                    return Err(invalid("invalid EROFS union"));
                }
                if journal.plan == "control-files" {
                    let paths = journal
                        .footprint
                        .iter()
                        .map(|p| p.get())
                        .collect::<io::Result<Vec<_>>>()?;
                    if paths
                        .iter()
                        .enumerate()
                        .any(|(i, p)| paths[i + 1..].iter().any(|q| overlaps(p, q)))
                    {
                        return Err(invalid("invalid ControlFiles footprint"));
                    }
                }
                if journal.plan != "erofs"
                    && journal.plan != "remove"
                    && journal.plan != "control-files"
                    && journal.footprint.len() != 1
                {
                    return Err(invalid("invalid tree footprint"));
                }
                if journal.committed
                    && (journal.intents.iter().any(|i| !i.completed && !i.cancelled)
                        || journal.disposals.iter().any(|d| !d.discarded))
                {
                    return Err(invalid("committed journal has unresolved effects"));
                }
                if journal.permissions.len() > 4096 {
                    return Err(limit());
                }
                for intent in &journal.permissions {
                    intent.path.get()?;
                    if intent.original.uid != self.namespace.uid
                        || intent.granted_mode & !0o7777 != 0
                        || intent.acl.as_ref().is_some_and(|a| a.len() > 131072)
                        || intent
                            .default_acl
                            .as_ref()
                            .is_some_and(|a| a.len() > 131072)
                    {
                        return Err(invalid("invalid permission intent"));
                    }
                }
                if journal.committed && journal.permissions.iter().any(|p| !p.restored) {
                    return Err(invalid("unrestored committed permission journal"));
                }
                let mut stages = std::collections::HashSet::new();
                for disposal in &journal.disposals {
                    disposal.path.get()?;
                    disposal.slot.get()?;
                    if !id_valid(&disposal.stage)
                        || !stages.insert(&disposal.stage)
                        || disposal.file.mode & libc::S_IFMT != libc::S_IFREG
                        || disposal.file.links != 1
                        || disposal.file.uid != self.namespace.uid
                        || disposal.parent.mode & libc::S_IFMT != libc::S_IFDIR
                    {
                        return Err(invalid("invalid stage disposal record"));
                    }
                }
                if !id_valid(&journal.id)
                    || journal.changes.len() > 512
                    || journal.artifacts.len() > 256
                    || journal.epoch > op.epoch
                {
                    return Err(limit());
                }
                for c in &journal.changes {
                    c.path.get()?;
                }
                for p in &journal.artifacts {
                    p.get()?;
                }
            }
        }
        let names = self.namespace.directory.names()?;
        if names.len() != allowed.len() || names.iter().any(|n| !allowed.contains(n)) {
            return Err(invalid("unknown or incomplete admission namespace"));
        }
        Ok(())
    }
    fn validate_resource_owner(
        &self,
        op: &Operation,
        owner: &ResourceOwner,
        historical: bool,
        endpoint: Option<&EndpointRecord>,
    ) -> io::Result<()> {
        if !id_valid(&owner.holder) {
            return Err(invalid("invalid resource holder"));
        }
        let member = op.holder_ids.contains(&owner.holder);
        if let Some(bound) = &owner.runtime {
            let runtime = self
                .catalog
                .runtimes
                .iter()
                .find(|r| r.id == bound.id && r.operation == op.id)
                .ok_or_else(|| invalid("resource runtime absent"))?;
            if runtime.epoch != bound.epoch
                || runtime.admission_holder.as_deref() != Some(owner.holder.as_str())
                || !matches!(
                    runtime.state,
                    RuntimeState::Active | RuntimeState::Retired | RuntimeState::Uncertain
                )
            {
                return Err(invalid("resource runtime/holder/epoch mismatch"));
            }
            let transferred = runtime.state == RuntimeState::Retired
                && self.catalog.endpoints.iter().any(|e| {
                    e.operation == op.id && e.owner.as_ref() == Some(owner) && e.hold_id.is_some()
                });
            if !member && !historical && !transferred {
                return Err(invalid(
                    "runtime resource holder no longer belongs to operation",
                ));
            }
        } else if !member && !historical {
            return Err(invalid("host resource holder absent"));
        }
        if let Some(endpoint) = endpoint {
            if owner
                .runtime
                .as_ref()
                .is_some_and(|r| r.epoch > endpoint.epoch)
            {
                return Err(invalid("endpoint predates its runtime"));
            }
        }
        Ok(())
    }
    pub(super) fn operation_sentinel(&self, op: &Operation) -> io::Result<File> {
        match self
            .namespace
            .directory
            .file(&format!("op-{}", op.id), false)
        {
            Ok(file) => Ok(file),
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && self
                        .catalog
                        .archives
                        .iter()
                        .any(|a| a.operation == op.id && a.sentinel == op.sentinel) =>
            {
                self.namespace.archive.file(&format!("op-{}", op.id), false)
            }
            Err(e) => Err(e),
        }
    }
    pub(super) fn namespace(&self) -> &Namespace {
        self.namespace
    }
    pub(super) fn operation(&self, id: &str) -> io::Result<&Operation> {
        self.catalog
            .operations
            .iter()
            .find(|op| op.id == id)
            .ok_or_else(|| invalid("missing operation"))
    }
    pub(super) fn operation_mut(&mut self, id: &str) -> io::Result<&mut Operation> {
        self.catalog
            .operations
            .iter_mut()
            .find(|op| op.id == id)
            .ok_or_else(|| invalid("missing operation"))
    }
    pub(super) fn save(&mut self) -> io::Result<()> {
        self.namespace.validate()?;
        // Retain the last complete snapshot before modifying the stable current
        // inode. A torn current snapshot still refuses; recovery never silently
        // falls back and thus cannot erase an uncertain writer's newest intent.
        let previous: Catalog = read_json(&self.file)?;
        write_json(&self.previous, &previous)?;
        write_json(&self.file, &self.catalog)
    }
    pub(super) fn check_conflicts(
        &self,
        own: Option<&str>,
        entries: &[Entry],
        transition: bool,
    ) -> io::Result<()> {
        if entries.len() > MAX_PATHS {
            return Err(limit());
        }
        for entry in entries {
            if overlaps(&entry.path.get()?, &self.namespace.directory.path) {
                return Err(invalid(
                    "coordination namespace cannot be an admitted payload",
                ));
            }
        }
        for op in &self.catalog.operations {
            if Some(op.id.as_str()) == own {
                continue;
            }
            let retained_sentinel = self.operation_sentinel(op)?;
            let sentinel_released = ofd(&retained_sentinel, true)?;
            if op.closed && sentinel_released {
                continue;
            }
            for a in entries {
                for b in &op.entries {
                    if !transition
                        && a.role == PathRole::Endpoint
                        && b.role == PathRole::Endpoint
                        && a.path == b.path
                        && super::endpoint::replaceable(self, self.namespace, op, &a.path.get()?)?
                    {
                        continue;
                    }
                    let ap = a.path.get()?;
                    let bp = b.path.get()?;
                    let left = if a.role == PathRole::Endpoint && b.role != PathRole::Endpoint {
                        ap.parent().ok_or_else(|| invalid("endpoint parent"))?
                    } else {
                        ap.as_path()
                    };
                    let right = if b.role == PathRole::Endpoint && a.role != PathRole::Endpoint {
                        bp.parent().ok_or_else(|| invalid("endpoint parent"))?
                    } else {
                        bp.as_path()
                    };
                    if overlaps(left, right)
                        && (transition
                            || op.transition
                            || op.uncertain
                            || a.role != PathRole::Strict
                            || b.role != PathRole::Strict)
                    {
                        return Err(busy());
                    }
                }
            }
            // Unknown writer history remains fenced; only independently proven
            // endpoint scope may bypass its own old endpoint entry.
            if !op.closed && sentinel_released {
                for a in entries {
                    for b in &op.entries {
                        if !overlaps(&a.path.get()?, &b.path.get()?) {
                            continue;
                        }
                        if !transition
                            && a.role == PathRole::Endpoint
                            && b.role == PathRole::Endpoint
                            && a.path == b.path
                            && super::endpoint::replaceable(
                                self,
                                self.namespace,
                                op,
                                &a.path.get()?,
                            )?
                        {
                            continue;
                        }
                        return Err(invalid("orphaned operation requires verified recovery"));
                    }
                }
            }
        }
        Ok(())
    }
}
pub(super) fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn provision(directory: &Directory, expected_uid: u32) -> io::Result<()> {
    if !directory.names()?.is_empty() {
        return Err(invalid("bootstrap refuses incomplete existing namespace"));
    }
    mkdir_at(&directory.file, "archive".as_ref(), 0o700)?;
    let archive = Directory::open(&directory.path.join("archive"))?;
    if uid() == 0 {
        syscall(unsafe { libc::fchown(archive.file.as_raw_fd(), expected_uid, !0) })?;
    }
    archive.file.sync_all()?;
    let mutex = directory.file("catalog.lock", true)?;
    let catalog = directory.file("catalog.json", true)?;
    let previous = directory.file("catalog.previous", true)?;
    for file in [&mutex, &catalog, &previous] {
        if uid() == 0 {
            syscall(unsafe { libc::fchown(file.as_raw_fd(), expected_uid, !0) })?;
        }
    }
    write_json(
        &catalog,
        &Catalog {
            version: 1,
            bases: Vec::new(),
            operations: Vec::new(),
            endpoints: Vec::new(),
            archives: Vec::new(),
            runtimes: Vec::new(),
        },
    )?;
    let initial: Catalog = read_json(&catalog)?;
    write_json(&previous, &initial)?;
    mutex.sync_all()?;
    let manifest = directory.file("manifest.json", true)?;
    if uid() == 0 {
        syscall(unsafe { libc::fchown(manifest.as_raw_fd(), expected_uid, !0) })?;
    }
    write_json(
        &manifest,
        &Manifest {
            version: 1,
            uid: expected_uid,
            namespace: Identity::of(&directory.file)?,
            mutex: Identity::of(&mutex)?,
            catalog: Identity::of(&catalog)?,
            previous: Identity::of(&previous)?,
            archive: Identity::of(&archive.file)?,
        },
    )?;
    directory.file.sync_all()
}
pub(crate) fn bootstrap_for_uid(expected_uid: u32) -> io::Result<()> {
    if uid() != 0 {
        return Err(invalid("bootstrap requires effective administrative uid0"));
    }
    let mut directory = Directory::open(Path::new("/var/lib"))?;
    directory.administrative()?;
    for component in ["iii", "worker-admission", "v1"] {
        match mkdir_at(&directory.file, component.as_ref(), 0o755) {
            Ok(()) => directory.file.sync_all()?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
        directory = Directory::open(&directory.path.join(component))?;
        directory.administrative()?;
    }
    let component = expected_uid.to_string();
    match mkdir_at(&directory.file, component.as_ref(), 0o700) {
        Ok(()) => {
            let user = open_at(
                &directory.file,
                component.as_ref(),
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?;
            syscall(unsafe { libc::fchown(user.as_raw_fd(), expected_uid, !0) })?;
            user.sync_all()?;
            directory.file.sync_all()?;
            provision(
                &Directory::open(&directory.path.join(&component))?,
                expected_uid,
            )?;
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            Namespace::open(&directory.path.join(component), expected_uid, true)?;
        }
        Err(e) => return Err(e),
    }
    Ok(())
}
pub(crate) fn check_installation() -> io::Result<()> {
    Namespace::production()?.transaction()?;
    Ok(())
}
