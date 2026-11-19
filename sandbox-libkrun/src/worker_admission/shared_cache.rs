// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Exact image-cache claims. Metadata writes are catalog-only on the warm path.
//! OCI selection, digest provenance and EROFS freshness remain caller decisions.
use super::{
    Held, Lease, MutationGuard, MutationPlan, OwnerReadGuard, PathRole, UseGuard,
    catalog::{CachePhase, CacheRecord, Entry, Operation, Transaction, overlaps},
    storage::*,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
const MAX_IMAGES: usize = 4;
pub(crate) struct CacheEntry<'a> {
    pub root: &'a Path,
}
pub(crate) struct CacheProbe {
    lease: Lease,
    consumer: Lease,
}
#[derive(Clone)]
pub(crate) struct CachePreparation {
    lease: Lease,
    consumer: Lease,
}
fn sibling(root: &Path, suffix: &str) -> io::Result<PathBuf> {
    let mut name = root
        .file_name()
        .ok_or_else(|| invalid("cache entry root missing"))?
        .to_os_string();
    name.push(suffix);
    Ok(root.with_file_name(name))
}
fn record(
    consumer: &Lease,
    entries: &[CacheEntry<'_>],
    phase: CachePhase,
) -> io::Result<CacheRecord> {
    if entries.is_empty() || entries.len() > MAX_IMAGES {
        return Err(limit());
    }
    consumer.revalidate()?;
    let tx = consumer.0.namespace.transaction()?;
    let op = tx.operation(&consumer.0.id)?;
    if consumer.0.holder != consumer.0.id
        || op.cache.is_some()
        || op.transition
        || op.closed
        || op.uncertain
        || op.holders != 1
        || !op.holds.is_empty()
        || tx.catalog.runtimes.iter().any(|r| r.operation == op.id)
        || tx.catalog.endpoints.iter().any(|e| e.operation == op.id)
    {
        return Err(invalid(
            "cache admission requires original pre-runtime lifecycle",
        ));
    }
    let mut roots = Vec::new();
    let mut files = Vec::new();
    let mut reads = Vec::new();
    for entry in entries {
        if !entry.root.is_absolute() {
            return Err(invalid("cache root must be absolute"));
        }
        let root = absolute(entry.root)?;
        let parent = Directory::open(
            root.parent()
                .ok_or_else(|| invalid("cache parent missing"))?,
        )?;
        let id = Identity::of(&parent.file)?;
        if id.uid != uid() || id.mode & 0o022 != 0 {
            return Err(invalid("cache parent must already be owned/protected"));
        }
        roots.push(HostPath::new(&root)?);
        reads.push(HostPath::new(&root)?);
        for suffix in [".erofs", ".digest", ".lock", ".erofs.lock", ".erofs.spool"] {
            let path = HostPath::new(&sibling(&root, suffix)?)?;
            if matches!(suffix, ".erofs" | ".digest") {
                reads.push(path.clone());
            }
            files.push(path);
        }
    }
    let all = roots
        .iter()
        .chain(&files)
        .map(HostPath::get)
        .collect::<io::Result<Vec<_>>>()?;
    if all
        .iter()
        .enumerate()
        .any(|(i, p)| all[i + 1..].iter().any(|q| overlaps(p, q)))
    {
        return Err(invalid("duplicate/overlapping exact cache entries"));
    }
    for existing in &op.entries {
        let p = existing.path.get()?;
        if all.iter().any(|q| overlaps(&p, q)) {
            return Err(invalid(
                "lifecycle already claims cache or broad cache ancestor",
            ));
        }
    }
    Ok(CacheRecord {
        phase,
        consumer: op.id.clone(),
        consumer_sentinel: op.sentinel.clone(),
        consumer_epoch: op.epoch,
        consumer_entries_sha256: super::recovery::digest(&op.entries)?,
        roots,
        files,
        reads,
        transferred_epoch: None,
        transferred_reads: Vec::new(),
    })
}
fn issue(consumer: Lease, cache: CacheRecord) -> io::Result<Lease> {
    let entries = if cache.phase == CachePhase::Probe {
        cache
            .reads
            .iter()
            .map(|p| super::entry(&p.get()?, PathRole::Strict))
            .collect::<io::Result<Vec<_>>>()?
    } else {
        let mut entries = cache
            .roots
            .iter()
            .map(|p| super::entry(&p.get()?, PathRole::MutableTree))
            .collect::<io::Result<Vec<_>>>()?;
        for p in &cache.files {
            entries.push(super::entry(&p.get()?, PathRole::MutableFile)?);
        }
        entries
    };
    let namespace = consumer.0.namespace.clone();
    let (id, sentinel) = namespace.start_with_cache(entries, None, false, Some(cache))?;
    Ok(Lease(Arc::new(Held {
        namespace,
        holder: id.clone(),
        id,
        sentinel,
    })))
}
impl Lease {
    pub(crate) fn probe_cache(&self, entries: &[CacheEntry<'_>]) -> io::Result<CacheProbe> {
        let cache = record(self, entries, CachePhase::Probe)?;
        Ok(CacheProbe {
            lease: issue(self.clone(), cache)?,
            consumer: self.clone(),
        })
    }
    pub(crate) fn prepare_cache(&self, entries: &[CacheEntry<'_>]) -> io::Result<CachePreparation> {
        let cache = record(self, entries, CachePhase::Preparing)?;
        Ok(CachePreparation {
            lease: issue(self.clone(), cache)?,
            consumer: self.clone(),
        })
    }
}
impl CacheProbe {
    pub(crate) fn exists(&self, path: &Path) -> io::Result<bool> {
        self.checkpoint()?;
        exact_read(&self.lease, path)?;
        super::present(&absolute(path)?)
    }
    pub(crate) fn use_path(&self, path: &Path) -> io::Result<UseGuard> {
        self.checkpoint()?;
        exact_read(&self.lease, path)?;
        self.lease.use_path(path)
    }
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        check(&self.lease, CachePhase::Probe)
    }
    pub(crate) fn handoff(self, consumer: &Lease, selected_root: &Path) -> io::Result<Lease> {
        transfer(&self.lease, &self.consumer, consumer, selected_root, None)
    }
    pub(crate) fn cancel(self) -> io::Result<()> {
        if Arc::strong_count(&self.lease.0) != 1 {
            return Err(invalid("provisional cache read still in use"));
        }
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        validate_phase(op, CachePhase::Probe)?;
        if !op.holds.is_empty() || !op.journals.is_empty() {
            return Err(busy());
        }
        op.entries.clear();
        op.closed = true;
        op.cache
            .as_mut()
            .ok_or_else(|| invalid("cache record absent"))?
            .phase = CachePhase::Cancelled;
        tx.save()
    }
    pub(crate) fn prepare(self) -> io::Result<CachePreparation> {
        // Only a provisional operation with no live read guard can be retired.
        // A competing producer may win this deliberate miss-path gap: the new
        // exclusive prep rechecks, and the caller must repeat semantic selection.
        if Arc::strong_count(&self.lease.0) != 1 {
            return Err(invalid("provisional read still in use"));
        }
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation(&self.lease.0.id)?;
        validate_phase(op, CachePhase::Probe)?;
        if !op.holds.is_empty() || !op.journals.is_empty() {
            return Err(busy());
        }
        let mut cache = op
            .cache
            .clone()
            .ok_or_else(|| invalid("cache record absent"))?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        op.entries.clear();
        op.closed = true;
        op.cache
            .as_mut()
            .ok_or_else(|| invalid("cache record absent"))?
            .phase = CachePhase::Cancelled;
        tx.save()?;
        drop(tx);
        cache.phase = CachePhase::Preparing;
        let lease = issue(self.consumer.clone(), cache)?;
        Ok(CachePreparation {
            lease,
            consumer: self.consumer.clone(),
        })
    }
}
impl CachePreparation {
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        check(&self.lease, CachePhase::Preparing)
    }
    pub(crate) fn exists(&self, path: &Path) -> io::Result<bool> {
        self.checkpoint()?;
        exact_read(&self.lease, path)?;
        super::present(&absolute(path)?)
    }
    pub(crate) fn use_path(&self, path: &Path) -> io::Result<UseGuard> {
        self.checkpoint()?;
        exact_read(&self.lease, path)?;
        self.lease.use_path(path)
    }
    pub(crate) fn begin_entry(&self, root: &Path) -> io::Result<MutationGuard> {
        self.checkpoint()?;
        let paths = slots(&self.lease, root)?;
        MutationGuard::begin_cache_entry(self.lease.clone(), root, &paths)
    }
    pub(crate) fn begin_erofs(&self, root: &Path) -> io::Result<MutationGuard> {
        self.checkpoint()?;
        slots(&self.lease, root)?;
        self.lease.begin(MutationPlan::Erofs {
            output: sibling(root, ".erofs")?,
            spool: sibling(root, ".erofs.spool")?,
        })
    }
    pub(crate) fn owner_read(&self, root: &Path) -> io::Result<OwnerReadGuard> {
        self.checkpoint()?;
        slots(&self.lease, root)?;
        OwnerReadGuard::begin(self.lease.clone(), root)
    }
    pub(crate) fn handoff(&self, consumer: &Lease, selected_root: &Path) -> io::Result<Lease> {
        transfer(&self.lease, &self.consumer, consumer, selected_root, None)
    }
    #[cfg(test)]
    pub(super) fn handoff_fault(
        &self,
        consumer: &Lease,
        root: &Path,
        at: &str,
    ) -> io::Result<Lease> {
        transfer(&self.lease, &self.consumer, consumer, root, Some(at))
    }
}
fn exact_read(lease: &Lease, path: &Path) -> io::Result<()> {
    let tx = lease.0.namespace.transaction()?;
    let path = HostPath::new(path)?;
    if !tx
        .operation(&lease.0.id)?
        .cache
        .as_ref()
        .is_some_and(|c| c.reads.contains(&path))
    {
        return Err(invalid("cache read outside exact roots/lower/digest set"));
    }
    Ok(())
}
fn slots(lease: &Lease, root: &Path) -> io::Result<Vec<PathBuf>> {
    let tx = lease.0.namespace.transaction()?;
    let op = tx.operation(&lease.0.id)?;
    validate_phase(op, CachePhase::Preparing)?;
    let cache = op
        .cache
        .as_ref()
        .ok_or_else(|| invalid("cache record absent"))?;
    let root = absolute(root)?;
    if !cache.roots.contains(&HostPath::new(&root)?) {
        return Err(invalid("cache root not registered for preparation"));
    }
    Ok(vec![
        root.clone(),
        sibling(&root, ".digest")?,
        sibling(&root, ".lock")?,
    ])
}
fn validate_phase(op: &Operation, phase: CachePhase) -> io::Result<()> {
    if op.closed || op.uncertain || op.cache.as_ref().is_none_or(|c| c.phase != phase) {
        return Err(invalid("cache capability closed, stale or uncertain"));
    }
    Ok(())
}
fn check(lease: &Lease, phase: CachePhase) -> io::Result<()> {
    lease.revalidate()?;
    let tx = lease.0.namespace.transaction()?;
    validate_phase(tx.operation(&lease.0.id)?, phase)
}
pub(super) fn managed_root(op: &Operation, path: &Path) -> bool {
    op.cache.as_ref().is_some_and(|c| {
        c.phase == CachePhase::Preparing && c.roots.iter().any(|p| p.get().is_ok_and(|p| p == path))
    }) && !op.closed
        && !op.uncertain
}
pub(super) fn validate_consumer(tx: &Transaction<'_>, cache: &CacheRecord) -> io::Result<()> {
    let op = tx.operation(&cache.consumer)?;
    if op.closed
        || op.uncertain
        || op.transition
        || op.cache.is_some()
        || op.holders != 1
        || op.sentinel != cache.consumer_sentinel
        || op.epoch != cache.consumer_epoch
        || super::recovery::digest(&op.entries)? != cache.consumer_entries_sha256
        || !op.holds.is_empty()
        || op.journals.iter().any(|j| !j.committed)
        || tx.catalog.runtimes.iter().any(|r| r.operation == op.id)
        || tx.catalog.endpoints.iter().any(|e| e.operation == op.id)
    {
        return Err(invalid("cache handoff consumer changed or already active"));
    }
    for entry in &op.entries {
        super::validate_entry(entry)?;
    }
    Ok(())
}
pub(super) fn validate_record(op: &Operation) -> io::Result<()> {
    let Some(c) = &op.cache else {
        return Ok(());
    };
    if op.transition
        || !id_valid(&c.consumer)
        || c.consumer == op.id
        || c.roots.is_empty()
        || c.roots.len() > MAX_IMAGES
        || c.files.len() != c.roots.len() * 5
        || c.reads.len() != c.roots.len() * 3
        || c.consumer_entries_sha256.len() != 64
    {
        return Err(invalid("invalid bounded cache operation"));
    }
    let mut files = Vec::new();
    let mut reads = Vec::new();
    for root in &c.roots {
        let root = root.get()?;
        reads.push(HostPath::new(&root)?);
        for suffix in [".erofs", ".digest", ".lock", ".erofs.lock", ".erofs.spool"] {
            let p = HostPath::new(&sibling(&root, suffix)?)?;
            if matches!(suffix, ".erofs" | ".digest") {
                reads.push(p.clone());
            }
            files.push(p);
        }
    }
    if files != c.files || reads != c.reads {
        return Err(invalid("cache exact slot binding mismatch"));
    }
    let paths = c
        .roots
        .iter()
        .chain(&c.files)
        .map(HostPath::get)
        .collect::<io::Result<Vec<_>>>()?;
    if paths
        .iter()
        .enumerate()
        .any(|(i, p)| paths[i + 1..].iter().any(|q| overlaps(p, q)))
    {
        return Err(invalid("overlapping cache declarations"));
    }
    if c.consumer_entries_sha256
        .bytes()
        .any(|b| !b.is_ascii_hexdigit())
    {
        return Err(invalid("invalid consumer snapshot digest"));
    }
    if c.phase == CachePhase::Probe
        && (!op.journals.is_empty()
            || op.entries.len() != c.reads.len()
            || op
                .entries
                .iter()
                .any(|e| e.role != PathRole::Strict || !c.reads.contains(&e.path)))
    {
        return Err(invalid("provisional cache read became writable"));
    }
    if matches!(c.phase, CachePhase::Transferred | CachePhase::Cancelled)
        && (!op.closed || !op.entries.is_empty() || !op.holds.is_empty() || op.uncertain)
    {
        return Err(invalid("closed cache operation retains live claims"));
    }
    if matches!(c.phase, CachePhase::Probe | CachePhase::Preparing)
        && (c.transferred_epoch.is_some() || !c.transferred_reads.is_empty())
    {
        return Err(invalid("active cache record has transfer metadata"));
    }
    if c.phase == CachePhase::Preparing {
        for root in &c.roots {
            if !op
                .entries
                .iter()
                .any(|e| &e.path == root && e.role == PathRole::MutableTree)
            {
                return Err(invalid("preparation root claim missing"));
            }
        }
        for path in &c.files {
            if !op
                .entries
                .iter()
                .any(|e| &e.path == path && e.role == PathRole::MutableFile)
            {
                return Err(invalid("preparation file claim missing"));
            }
        }
    }
    if c.phase != CachePhase::Transferred
        && (c.transferred_epoch.is_some() || !c.transferred_reads.is_empty())
    {
        return Err(invalid(
            "nontransferred cache record contains completion proof",
        ));
    }
    for entry in &c.transferred_reads {
        if entry.pins.len() > 128 {
            return Err(limit());
        }
        entry.path.get()?;
        for (p, _) in &entry.pins {
            p.get()?;
        }
    }
    if c.phase == CachePhase::Transferred
        && (c.transferred_epoch != Some(op.epoch)
            || c.transferred_reads.len() != c.reads.len()
            || c.transferred_reads
                .iter()
                .map(|e| e.path.get())
                .collect::<io::Result<std::collections::BTreeSet<_>>>()?
                .len()
                != c.reads.len()
            || c.transferred_reads
                .iter()
                .any(|e| e.role != PathRole::Strict || !c.reads.contains(&e.path)))
    {
        return Err(invalid("cache transfer receipt mismatch"));
    }
    Ok(())
}
fn transfer(
    prep: &Lease,
    bound_consumer: &Lease,
    consumer: &Lease,
    selected_root: &Path,
    fault: Option<&str>,
) -> io::Result<Lease> {
    if !Arc::ptr_eq(&bound_consumer.0, &consumer.0)
        || !Arc::ptr_eq(&prep.0.namespace, &consumer.0.namespace)
    {
        return Err(invalid(
            "cache transfer requires original consumer operation/context",
        ));
    }
    let mut tx = prep.0.namespace.transaction()?;
    let source = tx.operation(&prep.0.id)?;
    let cache = source
        .cache
        .clone()
        .ok_or_else(|| invalid("cache capability record absent"))?;
    if !matches!(cache.phase, CachePhase::Probe | CachePhase::Preparing) {
        return Err(invalid("cache transfer already consumed"));
    }
    validate_phase(source, cache.phase)?;
    validate_consumer(&tx, &cache)?;
    if Identity::of(&consumer.0.sentinel)? != cache.consumer_sentinel
        || consumer.0.holder != consumer.0.id
    {
        return Err(invalid("consumer sentinel or issuing holder changed"));
    }
    if source.sentinel != Identity::of(&prep.0.sentinel)?
        || source.holders != 1
        || source.holder_ids != vec![prep.0.holder.clone()]
        || !source.holds.is_empty()
        || source.journals.iter().any(|j| {
            !j.committed
                || j.intents.iter().any(|i| !i.completed || i.cancelled)
                || j.permissions
                    .iter()
                    .any(|p| !p.restored || !p.acl_before_grant)
                || j.disposals.iter().any(|d| !d.discarded)
        })
        || tx.catalog.runtimes.iter().any(|r| r.operation == source.id)
        || tx
            .catalog
            .endpoints
            .iter()
            .any(|e| e.operation == source.id)
    {
        return Err(invalid(
            "cache writer/guard/diagnostic/journal remains unresolved",
        ));
    }
    for entry in &source.entries {
        super::validate_entry(entry)?;
    }
    // A borrowed diagnostic FD can be duplicated by an independent integration
    // helper. Prove its flock was released too, without writing or creating it.
    // Never do this on the warm Probe path.
    let mut diagnostic_fences = Vec::new();
    if cache.phase == CachePhase::Preparing {
        for root in &cache.roots {
            for suffix in [".lock", ".erofs.lock"] {
                let path = sibling(&root.get()?, suffix)?;
                if super::present(&path)? {
                    let parent = Directory::open(
                        path.parent().ok_or_else(|| invalid("diagnostic parent"))?,
                    )?;
                    let file = open_at(
                        &parent.file,
                        path.file_name().ok_or_else(|| invalid("diagnostic leaf"))?,
                        libc::O_RDWR | libc::O_NONBLOCK,
                        0,
                    )?;
                    let identity = Identity::of(&file)?;
                    if identity.uid != uid()
                        || identity.mode & libc::S_IFMT != libc::S_IFREG
                        || identity.mode & 0o022 != 0
                        || identity.links != 1
                    {
                        return Err(invalid("unsafe cache diagnostic completion"));
                    }
                    let entry = source
                        .entries
                        .iter()
                        .find(|e| e.path.get().is_ok_and(|p| p == path))
                        .ok_or_else(|| invalid("diagnostic was not admitted"))?;
                    if !entry
                        .pins
                        .iter()
                        .any(|(p, id)| p.get().is_ok_and(|p| p == path) && id.same_inode(&identity))
                    {
                        return Err(invalid("diagnostic inode changed before handoff"));
                    }
                    flock(&file)?;
                    diagnostic_fences.push(file);
                }
            }
        }
    }
    let selected = absolute(selected_root)?;
    if !cache.roots.contains(&HostPath::new(&selected)?) {
        return Err(invalid("selected cache root outside candidate set"));
    }
    for (path, directory) in [
        (selected.clone(), true),
        (sibling(&selected, ".erofs")?, false),
    ] {
        let meta = std::fs::symlink_metadata(&path)?;
        if (directory && !meta.is_dir())
            || (!directory
                && (!meta.is_file() || std::os::unix::fs::MetadataExt::nlink(&meta) != 1))
        {
            return Err(invalid("selected source/lower/digest has wrong type"));
        }
    }
    // The digest sidecar may legitimately be absent for an unpinned historical
    // hit. Its absence remains a Strict selection claim; only the caller can
    // decide whether its reference requires digest evidence before handoff.
    let entries = cache
        .reads
        .iter()
        .map(|p| super::entry(&p.get()?, PathRole::Strict))
        .collect::<io::Result<Vec<_>>>()?;
    for read in &entries {
        let previous = source
            .entries
            .iter()
            .find(|e| e.path == read.path)
            .ok_or_else(|| invalid("handoff read was not protected by preparation"))?;
        if previous.pins.len() != read.pins.len()
            || !previous.pins.iter().all(|(p, id)| {
                read.pins.iter().any(|(q, current)| {
                    p == q
                        && id.same_inode(current)
                        && (id.mode & libc::S_IFMT == libc::S_IFDIR || id.links == current.links)
                })
            })
        {
            return Err(invalid("cache identity changed during handoff validation"));
        }
    }
    super::inspect_entries(&entries, &tx.catalog.bases, false)?;
    let old_entries = source.entries.clone();
    let old_closed = source.closed;
    // Temporarily hide only our own prep claims in this in-memory transaction
    // while checking the full target union. Nothing is persisted or unlocked yet.
    {
        let op = tx.operation_mut(&prep.0.id)?;
        op.entries.clear();
        op.closed = true;
    }
    let mut combined = tx.operation(&consumer.0.id)?.entries.clone();
    combined.extend(entries.clone());
    let checked = tx.check_conflicts(Some(&consumer.0.id), &combined, false);
    {
        let op = tx.operation_mut(&prep.0.id)?;
        op.entries = old_entries;
        op.closed = old_closed;
    }
    checked?;
    if fault == Some("before-save") {
        return Err(io::Error::other("injected cache handoff before save"));
    }
    let epoch = tx.operation(&prep.0.id)?.epoch;
    let prep_sentinel = tx.operation(&prep.0.id)?.sentinel.clone();
    let target = tx.operation_mut(&consumer.0.id)?;
    if target.cache_reads.len() >= 16 {
        return Err(limit());
    }
    target.cache_reads.push(super::catalog::CacheReadReceipt {
        preparation: prep.0.id.clone(),
        preparation_sentinel: prep_sentinel,
        epoch,
        entries: entries.clone(),
    });
    target.entries = combined;
    let op = tx.operation_mut(&prep.0.id)?;
    op.entries.clear();
    op.closed = true;
    let record = op
        .cache
        .as_mut()
        .ok_or_else(|| invalid("cache record absent"))?;
    record.phase = CachePhase::Transferred;
    record.transferred_epoch = Some(epoch);
    record.transferred_reads = entries;
    #[cfg(test)]
    if fault == Some("partial-save") {
        tx.fault_partial_save()?;
    }
    // ONE catalog update both installs read coverage and retires only this prep.
    // Failure never returns a consumer. If bytes reached the current catalog but
    // durability acknowledgement failed, mark the consumer uncertain when the
    // still-held transaction can persist that fact. Torn records remain corrupt,
    // never repaired from catalog.previous or returned as a successful handoff.
    if let Err(error) = tx.save() {
        tx.operation_mut(&consumer.0.id)?.uncertain = true;
        let _ = tx.save();
        return Err(error);
    }
    if fault == Some("after-save") {
        // Conservatively flag the consumer if completion could not be returned.
        // Its Strict coverage stays installed; there is no writer-reopen gap.
        tx.operation_mut(&consumer.0.id)?.uncertain = true;
        tx.save()?;
        return Err(io::Error::other("injected ambiguous cache handoff result"));
    }
    Ok(consumer.clone())
}
#[cfg(test)]
#[path = "shared_cache_tests.rs"]
mod tests;
