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
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum HoldKind {
    Read,
    Mutation,
    Endpoint,
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
    pub epoch: u64,
    pub committed: bool,
    pub changes: Vec<Change>,
    pub artifacts: Vec<HostPath>,
    pub disposals: Vec<Disposal>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disposal {
    pub stage: String,
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
    pub closed: bool,
    pub uncertain: bool,
    pub epoch: u64,
    pub entries: Vec<Entry>,
    pub holds: Vec<Hold>,
    pub journals: Vec<Journal>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    version: u32,
    pub bases: Vec<HostPath>,
    pub operations: Vec<Operation>,
    pub endpoints: Vec<EndpointRecord>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EndpointRecord {
    pub path: HostPath,
    pub identity: Identity,
    pub operation: String,
    pub epoch: u64,
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
    uid: u32,
    manifest: Identity,
    mutex: Identity,
    catalog: Identity,
    previous: Identity,
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
        let ns = Self {
            directory,
            uid: expected_uid,
            manifest: Identity::of(&mf)?,
            mutex: manifest.mutex,
            catalog: manifest.catalog,
            previous: manifest.previous,
            administrative,
        };
        ns.transaction()?;
        Ok(ns)
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        self.directory.revalidate()?;
        self.directory.private(self.uid)?;
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
        tx.collect_closed()?;
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
            closed: false,
            uncertain: false,
            epoch: 0,
            entries,
            holds: Vec::new(),
            journals: Vec::new(),
        });
        tx.save()?;
        self.directory.file.sync_all()?;
        Ok((id, file))
    }
    pub(super) fn import(self: &Arc<Self>, file: &File) -> io::Result<String> {
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
        tx.save()?;
        Ok(record.id)
    }
    pub(super) fn release(&self, id: &str) -> io::Result<()> {
        let mut tx = self.transaction()?;
        let op = tx.operation_mut(id)?;
        op.holders = op
            .holders
            .checked_sub(1)
            .ok_or_else(|| invalid("holder underflow"))?;
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
        let mut endpoint_paths = std::collections::HashSet::new();
        for endpoint in &self.catalog.endpoints {
            let path = endpoint.path.get()?;
            if !endpoint_paths.insert(path)
                || endpoint.identity.mode & libc::S_IFMT != libc::S_IFSOCK
                || endpoint.identity.uid != self.namespace.uid
                || !self
                    .catalog
                    .operations
                    .iter()
                    .any(|op| op.id == endpoint.operation && endpoint.epoch <= op.epoch)
            {
                return Err(invalid("invalid endpoint receipt"));
            }
        }
        let mut allowed = vec![
            "manifest.json".to_string(),
            "catalog.lock".to_string(),
            "catalog.json".to_string(),
            "catalog.previous".to_string(),
        ];
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
            {
                return Err(invalid("invalid catalog operation bounds"));
            }
            let n = format!("op-{}", op.id);
            allowed.push(n.clone());
            let sentinel_file = self.namespace.directory.file(&n, false)?;
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
            for hold in &op.holds {
                hold.path.get()?;
                if !id_valid(&hold.id) || hold.epoch > op.epoch {
                    return Err(invalid("invalid hold"));
                }
            }
            let mut journals = std::collections::HashSet::new();
            for journal in &op.journals {
                journal.root.get()?;
                if !journals.insert(&journal.id)
                    || !["create", "adopt", "reclone", "cache", "erofs", "image-pair"]
                        .contains(&journal.plan.as_str())
                    || journal.disposals.len() > 256
                {
                    return Err(invalid("invalid mutation plan/id"));
                }
                let mut stages = std::collections::HashSet::new();
                for disposal in &journal.disposals {
                    disposal.path.get()?;
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
    fn collect_closed(&mut self) -> io::Result<()> {
        let closed: Vec<_> = self
            .catalog
            .operations
            .iter()
            .filter(|op| {
                op.closed
                    && op.journals.is_empty()
                    && !self.catalog.endpoints.iter().any(|e| e.operation == op.id)
            })
            .map(|op| op.id.clone())
            .collect();
        for id in closed {
            let n = format!("op-{id}");
            let file = self.namespace.directory.file(&n, false)?;
            // A caller may still be dropping its final descriptor. Never wait.
            if sentinel(&file).is_err() {
                continue;
            }
            self.catalog.operations.retain(|op| op.id != id);
            self.save()?;
            unlink_at(&self.namespace.directory.file, n.as_ref())?;
            self.namespace.directory.file.sync_all()?;
        }
        Ok(())
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
            let retained_sentinel = self
                .namespace
                .directory
                .file(&format!("op-{}", op.id), false)?;
            let sentinel_released = ofd(&retained_sentinel, true)?;
            if op.closed && sentinel_released {
                continue;
            }
            let endpoint_restart = !transition
                && !op.transition
                && !op.uncertain
                && op.journals.is_empty()
                && !op.entries.is_empty()
                && op.entries.iter().all(|e| e.role == PathRole::Endpoint)
                && entries.iter().all(|a| {
                    a.role == PathRole::Endpoint && op.entries.iter().any(|b| a.path == b.path)
                })
                && op.entries.iter().all(|e| {
                    self.catalog
                        .endpoints
                        .iter()
                        .any(|r| r.operation == op.id && r.path == e.path)
                });
            if endpoint_restart {
                let sentinel_file = self
                    .namespace
                    .directory
                    .file(&format!("op-{}", op.id), false)?;
                if ofd(&sentinel_file, true)? {
                    continue;
                }
            }
            for a in entries {
                for b in &op.entries {
                    if overlaps(&a.path.get()?, &b.path.get()?)
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
            // Dead sentinel with a nonclosed record is unknown writer history,
            // not evidence that migration is safe. Never reap that record here.
            let file = self
                .namespace
                .directory
                .file(&format!("op-{}", op.id), false)?;
            if !op.closed
                && ofd(&file, true)?
                && entries.iter().any(|a| {
                    op.entries
                        .iter()
                        .any(|b| match (a.path.get(), b.path.get()) {
                            (Ok(a), Ok(b)) => overlaps(&a, &b),
                            _ => true,
                        })
                })
            {
                return Err(invalid("orphaned operation requires verified recovery"));
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
