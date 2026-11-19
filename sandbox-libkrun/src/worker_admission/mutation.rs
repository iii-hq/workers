// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Owned mutations journal intent before touching data. No generic identity refresh.
use super::{
    Lease, MutationPlan, PathRole,
    catalog::{Change, HoldKind, Journal, overlaps},
    pins,
    storage::*,
};
use std::{
    fs::File,
    io,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        unix::fs::{FileExt, MetadataExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
pub(crate) struct EffectReceipt {
    _operation: String,
    _journal: String,
    _epoch: u64,
    _changes: Vec<Change>,
    _disposals: Vec<super::catalog::Disposal>,
}
pub(crate) struct MutationGuard {
    lease: Lease,
    id: String,
    root: PathBuf,
    footprint: Vec<PathBuf>,
    spool: Option<PathBuf>,
    plan: String,
    epoch: u64,
    expected: Vec<(HostPath, Identity)>,
    changes: Vec<Change>,
    stages: Vec<String>,
    committed: bool,
    diagnostics: Arc<DiagnosticState>,
    diagnostic_paths: Vec<PathBuf>,
    artifacts_started: bool,
    #[cfg(test)]
    discard_fault: Option<&'static str>,
    #[cfg(test)]
    publish_fault: Option<&'static str>,
}
pub(crate) struct StagedFile {
    owner: String,
    operation: String,
    namespace: Arc<super::catalog::Namespace>,
    token: String,
    target: PathBuf,
    path: PathBuf,
    parent: Directory,
    file: File,
    identity: Identity,
    cache_writer_fence: bool,
}
pub(crate) struct StagedTree {
    owner: String,
    operation: String,
    namespace: Arc<super::catalog::Namespace>,
    token: String,
    target: PathBuf,
    path: PathBuf,
    parent: Directory,
    directory: Directory,
    identity: Identity,
}
impl StagedFile {
    pub(crate) fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
impl StagedTree {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
fn settle_cache_stage(stage: StagedFile) -> io::Result<StagedFile> {
    if !stage.cache_writer_fence {
        return Ok(stage);
    }
    let StagedFile {
        owner,
        operation,
        namespace,
        token,
        target,
        path,
        parent,
        file,
        identity,
        cache_writer_fence,
    } = stage;
    parent.revalidate()?;
    if Identity::of(&file)? != identity {
        return Err(invalid("cache stage inode changed"));
    }
    // Clones share the original OFD locks. Closing only our descriptor cannot
    // retire a still-owned writer. Independent nonblocking lock must succeed.
    drop(file);
    let file = open_at(
        &parent.file,
        path.file_name().ok_or_else(|| invalid("stage leaf"))?,
        libc::O_RDWR | libc::O_NONBLOCK,
        0,
    )?;
    if Identity::of(&file)? != identity {
        return Err(invalid("cache stage substituted before completion"));
    }
    sentinel(&file)?;
    Ok(StagedFile {
        owner,
        operation,
        namespace,
        token,
        target,
        path,
        parent,
        file,
        identity,
        cache_writer_fence,
    })
}
fn fingerprint(path: &Path) -> io::Result<Option<Identity>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            if m.file_type().is_symlink()
                || (!m.is_file() && !m.is_dir())
                || (m.is_file() && m.nlink() != 1)
            {
                return Err(invalid("unsafe mutation target type/link"));
            }
            Ok(Some(Identity::metadata(&m)))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn same(a: &Option<Identity>, b: &Option<Identity>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_inode(b),
        _ => false,
    }
}
fn no_replace(from: &Directory, source: &Path, to: &Directory, target: &Path) -> io::Result<()> {
    from.revalidate()?;
    to.revalidate()?;
    let source = name(source.file_name().ok_or_else(|| invalid("rename source"))?)?;
    let target = name(target.file_name().ok_or_else(|| invalid("rename target"))?)?;
    #[cfg(target_os = "linux")]
    {
        syscall(unsafe {
            libc::renameat2(
                from.file.as_raw_fd(),
                source.as_ptr(),
                to.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic NOREPLACE required",
        ))
    }
}
impl MutationGuard {
    pub(super) fn begin(lease: Lease, plan: MutationPlan) -> io::Result<Self> {
        if matches!(plan, MutationPlan::Remove { .. }) {
            return Err(invalid(
                "Remove requires core-issued retirement or never-spawned receipt",
            ));
        }
        Self::begin_inner(lease, plan, None, None)
    }
    pub(super) fn begin_cache_entry(
        lease: Lease,
        root: &Path,
        paths: &[PathBuf],
    ) -> io::Result<Self> {
        let root = absolute(root)?;
        let mut digest = root
            .file_name()
            .ok_or_else(|| invalid("cache root name"))?
            .to_os_string();
        digest.push(".digest");
        let mut lock = root
            .file_name()
            .ok_or_else(|| invalid("cache root name"))?
            .to_os_string();
        lock.push(".lock");
        let exact = vec![
            root.clone(),
            root.with_file_name(digest),
            root.with_file_name(lock),
        ];
        if paths != exact.as_slice() {
            return Err(invalid(
                "cache-entry footprint must be exact root/digest/lock",
            ));
        }
        Self::begin_inner(
            lease,
            MutationPlan::Cache {
                root: root.to_path_buf(),
            },
            None,
            Some(paths.to_vec()),
        )
    }
    // Only the private receipt module calls this; no crate-visible metadata
    // constructor grants removal. Validation and hold acquisition share one tx.
    pub(super) fn begin_authorized(
        lease: Lease,
        plan: MutationPlan,
        epoch: u64,
        bindings: &[super::catalog::Entry],
        lineage: &super::lineage::Anchor,
        runtime: &str,
        sentinel: &Identity,
        never_spawned: bool,
    ) -> io::Result<Self> {
        Self::begin_inner(
            lease,
            plan,
            Some((epoch, bindings, lineage, runtime, sentinel, never_spawned)),
            None,
        )
    }
    fn begin_inner(
        lease: Lease,
        plan: MutationPlan,
        authority: Option<(
            u64,
            &[super::catalog::Entry],
            &super::lineage::Anchor,
            &str,
            &Identity,
            bool,
        )>,
        cache_footprint: Option<Vec<PathBuf>>,
    ) -> io::Result<Self> {
        let removing = matches!(&plan, MutationPlan::Remove { .. });
        let control = matches!(&plan, MutationPlan::ControlFiles { .. });
        let (plan, remove_targets) = match plan {
            MutationPlan::ControlFiles { files } => {
                if files.is_empty() || files.len() > 256 || files.iter().any(|p| !p.is_absolute()) {
                    return Err(invalid(
                        "ControlFiles requires a bounded nonempty absolute set",
                    ));
                }
                let mut exact = files
                    .iter()
                    .map(|p| absolute(p))
                    .collect::<io::Result<Vec<_>>>()?;
                exact.sort();
                if exact
                    .iter()
                    .enumerate()
                    .any(|(i, a)| exact[i + 1..].iter().any(|b| overlaps(a, b)))
                {
                    return Err(invalid("control files must be distinct and nonoverlapping"));
                }
                for path in &exact {
                    let parent =
                        Directory::open(path.parent().ok_or_else(|| invalid("control parent"))?)?;
                    let id = Identity::of(&parent.file)?;
                    if id.uid != uid() || id.mode & 0o022 != 0 {
                        return Err(invalid("control parent must be owned/protected"));
                    }
                }
                (
                    MutationPlan::Create {
                        root: exact[0].clone(),
                    },
                    Some(exact),
                )
            }
            MutationPlan::Remove { root, targets } => {
                let root = absolute(&root)?;
                if targets.is_empty() || targets.len() > 256 {
                    return Err(limit());
                }
                let mut exact = targets
                    .iter()
                    .map(|p| absolute(p))
                    .collect::<io::Result<Vec<_>>>()?;
                exact.sort();
                exact.dedup();
                if exact.iter().any(|p| !p.starts_with(&root))
                    || exact
                        .iter()
                        .enumerate()
                        .any(|(i, a)| exact[i + 1..].iter().any(|b| overlaps(a, b)))
                {
                    return Err(invalid("removal targets overlap or exceed root"));
                }
                (MutationPlan::Create { root }, Some(exact))
            }
            other => (other, None),
        };
        let (root, spool, mut label, mut role) = match plan {
            MutationPlan::Create { root } => (root, None, "create", PathRole::MutableTree),
            MutationPlan::Adopt { root } => (root, None, "adopt", PathRole::MutableTree),
            MutationPlan::Reclone { root } => (root, None, "reclone", PathRole::MutableTree),
            MutationPlan::Cache { root } => (root, None, "cache", PathRole::MutableTree),
            MutationPlan::Erofs { output, spool } => {
                if !output.is_absolute() || !spool.is_absolute() {
                    return Err(invalid("EROFS requires absolute output and spool"));
                }
                (
                    output,
                    Some(absolute(&spool)?),
                    "erofs",
                    PathRole::MutableFile,
                )
            }
            MutationPlan::ImagePair { directory } => {
                (directory, None, "image-pair", PathRole::MutableTree)
            }
            MutationPlan::Remove { .. } | MutationPlan::ControlFiles { .. } => {
                return Err(invalid("unresolved exact plan"));
            }
        };
        let root = absolute(&root)?;
        let cache_entry = cache_footprint.is_some();
        let mut footprint = remove_targets.unwrap_or_else(|| vec![root.clone()]);
        if removing {
            label = "remove";
        }
        if let Some(exact) = cache_footprint {
            label = "cache-entry";
            footprint = exact;
        }
        if control {
            label = "control-files";
            role = PathRole::MutableFile;
        }
        if let Some(spool) = &spool {
            if overlaps(&root, spool) {
                return Err(invalid("EROFS output/spool overlap"));
            }
            footprint.push(spool.clone());
        }
        lease.revalidate()?;
        let mut tx = lease.0.namespace.transaction()?;
        lease.validate_continuation(&tx)?;
        if removing {
            let (epoch, bindings, lineage, runtime, sentinel, never_spawned) =
                authority.ok_or_else(|| invalid("missing cleanup receipt"))?;
            let op = tx.operation(&lease.0.id)?;
            if &op.sentinel != sentinel
                || tx
                    .catalog
                    .runtimes
                    .iter()
                    .filter(|r| r.operation == lease.0.id)
                    .map(|r| r.epoch)
                    .max()
                    != Some(epoch)
                || !op.holds.is_empty()
                || op.holder_ids.iter().any(|id| id != &lease.0.holder)
                || op.journals.iter().any(|j| !j.committed)
            {
                return Err(invalid(
                    "stale cleanup receipt/layout or unresolved effects",
                ));
            }
            // Frozen receipt identities evolve only through complete,
            // ordered SAME-operation journals; never refresh from current paths.
            super::lineage::validate(lineage, bindings, op, &footprint, never_spawned)?;
            let required = if never_spawned {
                super::catalog::RuntimeState::Cancelled
            } else {
                super::catalog::RuntimeState::Retired
            };
            if !tx
                .catalog
                .runtimes
                .iter()
                .any(|r| r.id == runtime && r.operation == lease.0.id && r.state == required)
                || tx
                    .catalog
                    .runtimes
                    .iter()
                    .filter(|r| r.operation == lease.0.id)
                    .any(|r| {
                        !matches!(
                            r.state,
                            super::catalog::RuntimeState::Retired
                                | super::catalog::RuntimeState::Cancelled
                        )
                    })
            {
                return Err(invalid("cleanup writer proof no longer current"));
            }
            if tx.catalog.endpoints.iter().any(|e| {
                e.operation == lease.0.id && e.state != super::catalog::EndpointState::Retired
            }) {
                return Err(invalid("endpoint must retire before removal"));
            }
        }
        let op = tx.operation_mut(&lease.0.id)?;
        if cache_entry && !super::shared_cache::managed_root(op, &root) {
            return Err(invalid(
                "cache entry plan has no private preparation capability",
            ));
        }
        if op.uncertain
            || op.closed
            || op.holds.len() + footprint.len() > 256
            || op.journals.len() >= 256
        {
            return Err(busy());
        }
        let identities = footprint
            .iter()
            .map(|p| fingerprint(p))
            .collect::<io::Result<Vec<_>>>()?;
        if identities
            .iter()
            .flatten()
            .any(|id| id.uid != uid() || id.mode & 0o022 != 0)
        {
            return Err(invalid("mutable footprint is not owned/protected"));
        }
        if (spool.is_some() || control)
            && identities
                .iter()
                .flatten()
                .any(|id| id.mode & libc::S_IFMT != libc::S_IFREG)
        {
            return Err(invalid("EROFS requires regular file targets"));
        }
        if identities.iter().enumerate().any(|(i, a)| {
            a.as_ref().is_some_and(|a| {
                identities[i + 1..]
                    .iter()
                    .flatten()
                    .any(|b| a.dev == b.dev && a.ino == b.ino)
            })
        }) {
            return Err(invalid("mutation footprints alias one inode"));
        }
        for path in &footprint {
            if !op.entries.iter().any(|e| {
                (e.role == role || ((removing || cache_entry) && e.role == PathRole::MutableFile))
                    && e.path.get().is_ok_and(|p| {
                        p == *path
                            || (spool.is_none()
                                && role == PathRole::MutableTree
                                && path.starts_with(p))
                    })
            }) {
                return Err(invalid("exact mutable footprint not admitted"));
            }
            for e in &op.entries {
                if e.role != PathRole::Strict {
                    continue;
                }
                if overlaps(&e.path.get()?, path)
                    || e.pins.iter().any(|(_, pin)| {
                        identities
                            .iter()
                            .flatten()
                            .any(|id| pin.dev == id.dev && pin.ino == id.ino)
                    })
                {
                    return Err(invalid("Strict input aliases mutation"));
                }
            }
            if op
                .holds
                .iter()
                .any(|h| h.path.get().is_ok_and(|p| overlaps(&p, path)))
            {
                return Err(busy());
            }
        }
        let id = new_id();
        op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
        let epoch = op.epoch;
        let mut expected = Vec::new();
        for e in &op.entries {
            for pin in &e.pins {
                if footprint
                    .iter()
                    .any(|p| pin.0.get().is_ok_and(|q| overlaps(&q, p)))
                    && !expected.iter().any(|(p, _)| p == &pin.0)
                {
                    expected.push(pin.clone());
                }
            }
        }
        for path in &footprint {
            op.holds.push(super::catalog::Hold {
                id: id.clone(),
                path: HostPath::new(path)?,
                kind: HoldKind::Mutation,
                epoch,
                owner: Some(super::catalog::ResourceOwner {
                    holder: lease.0.holder.clone(),
                    runtime: None,
                }),
            });
        }
        op.journals.push(Journal {
            id: id.clone(),
            plan: label.into(),
            root: HostPath::new(&root)?,
            footprint: footprint
                .iter()
                .map(|p| HostPath::new(p))
                .collect::<io::Result<_>>()?,
            intents: Vec::new(),
            permissions: Vec::new(),
            epoch,
            committed: false,
            changes: Vec::new(),
            artifacts: Vec::new(),
            disposals: Vec::new(),
        });
        tx.save()?;
        drop(tx);
        Ok(Self {
            lease,
            id,
            root,
            footprint,
            spool,
            plan: label.into(),
            epoch,
            expected,
            changes: Vec::new(),
            stages: Vec::new(),
            committed: false,
            diagnostics: Arc::new(DiagnosticState {
                handles: AtomicUsize::new(0),
                failed: AtomicBool::new(false),
            }),
            diagnostic_paths: Vec::new(),
            artifacts_started: false,
            #[cfg(test)]
            discard_fault: None,
            #[cfg(test)]
            publish_fault: None,
        })
    }
    fn target(&self, path: &Path) -> io::Result<PathBuf> {
        let path = absolute(path)?;
        if self.plan == "cache-entry" {
            if path != self.root && !self.footprint.contains(&path) {
                return Err(invalid(
                    "cache preparation cannot mutate arbitrary descendants or siblings",
                ));
            }
            return Ok(path);
        }
        if !self.footprint.iter().any(|p| {
            path == *p
                || (self.plan != "erofs"
                    && self.plan != "remove"
                    && self.plan != "control-files"
                    && path.starts_with(p))
        }) {
            return Err(invalid("mutation exceeds declared footprint"));
        }
        Ok(path)
    }
    fn checkpoint_inner(&self) -> io::Result<()> {
        if self.diagnostics.failed.load(Ordering::Acquire) {
            return Err(invalid("diagnostic checkpoint is terminal"));
        }
        let tx = self.lease.0.namespace.transaction()?;
        self.lease.validate_continuation(&tx)?;
        let op = tx.operation(&self.lease.0.id)?;
        if op.uncertain
            || op.closed
            || !op
                .holds
                .iter()
                .any(|h| h.id == self.id && h.kind == HoldKind::Mutation)
            || !op
                .journals
                .iter()
                .any(|j| j.id == self.id && j.epoch == self.epoch && !j.committed)
        {
            return Err(invalid("mutation no longer owns its footprint"));
        }
        for (path, expected) in &self.expected {
            let path = path.get()?;
            let current = fingerprint(&path)?
                .ok_or_else(|| invalid("pinned mutation ancestor disappeared"))?;
            if !expected.same_inode(&current) {
                return Err(invalid("external substitution during mutation"));
            }
            // Validate the entire chain, not only a final lstat.
            pins(&path, PathRole::Strict)?;
        }
        for change in &self.changes {
            if !same(&change.new, &fingerprint(&change.path.get()?)?) {
                return Err(invalid("owned publication replaced externally"));
            }
        }
        super::inspect_entries(&op.entries, &tx.catalog.bases, false)
    }
    fn journal(&self, artifact: Option<&Path>, change: Option<Change>) -> io::Result<()> {
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        if op.uncertain {
            return Err(invalid("uncertain mutation"));
        }
        if let Some(path) = artifact {
            let encoded = HostPath::new(path)?;
            if !op.entries.iter().any(|e| e.path == encoded) {
                if op.entries.len() >= super::catalog::MAX_PATHS || op.holds.len() >= 256 {
                    return Err(limit());
                }
                // Stage and preserved-prior names are exact core-generated
                // siblings; add their intervals BEFORE creating/moving bytes.
                op.entries.push(super::entry(path, PathRole::MutableTree)?);
                op.holds.push(super::catalog::Hold {
                    id: self.id.clone(),
                    path: encoded,
                    kind: HoldKind::Mutation,
                    epoch: self.epoch,
                    owner: Some(super::catalog::ResourceOwner {
                        holder: self.lease.0.holder.clone(),
                        runtime: None,
                    }),
                });
            }
        }
        let journal = op
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("missing mutation journal"))?;
        if journal.artifacts.len() >= 256 || journal.changes.len() >= 512 {
            return Err(limit());
        }
        if let Some(path) = artifact {
            journal.artifacts.push(HostPath::new(path)?);
        }
        if let Some(change) = change {
            journal.changes.push(change);
        }
        let entries = tx.operation(&self.lease.0.id)?.entries.clone();
        tx.check_conflicts(Some(&self.lease.0.id), &entries, false)?;
        tx.save()
    }
    fn intent(
        &self,
        kind: super::catalog::EffectKind,
        from: Option<&Path>,
        to: &Path,
        old: Option<Identity>,
        new: Option<Identity>,
        prior: Option<&Path>,
    ) -> io::Result<String> {
        let to_parent = Directory::open(to.parent().ok_or_else(|| invalid("intent parent"))?)?;
        let from_parent = from
            .map(|p| Directory::open(p.parent().ok_or_else(|| invalid("intent source parent"))?))
            .transpose()?;
        let id = new_id();
        let mut tx = self.lease.0.namespace.transaction()?;
        let journal = tx
            .operation_mut(&self.lease.0.id)?
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("intent journal missing"))?;
        if journal.intents.len() >= 512 {
            return Err(limit());
        }
        journal.intents.push(super::catalog::EffectIntent {
            id: id.clone(),
            kind,
            from: from.map(HostPath::new).transpose()?,
            to: HostPath::new(to)?,
            from_parent: from_parent
                .as_ref()
                .map(|p| Identity::of(&p.file))
                .transpose()?,
            to_parent: Identity::of(&to_parent.file)?,
            old,
            new,
            prior: prior.map(HostPath::new).transpose()?,
            completed: false,
            cancelled: false,
        });
        tx.save()?;
        Ok(id)
    }
    fn complete_intent(&self, id: &str, new: Option<Identity>) -> io::Result<()> {
        let mut tx = self.lease.0.namespace.transaction()?;
        let journal = tx
            .operation_mut(&self.lease.0.id)?
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("intent journal missing"))?;
        let intent = journal
            .intents
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| invalid("intent missing"))?;
        if let Some(new) = new {
            intent.new = Some(new);
        }
        intent.completed = true;
        tx.save()
    }
    fn changed(
        &mut self,
        path: &Path,
        old: Option<Identity>,
        new: Option<Identity>,
    ) -> io::Result<()> {
        let change = Change {
            path: HostPath::new(path)?,
            old,
            new: new.clone(),
        };
        self.journal(None, Some(change.clone()))?;
        if self.expected.len() >= 8192 || self.changes.len() >= 512 {
            return Err(limit());
        }
        self.expected.retain(|(p, _)| p != &change.path);
        if let Some(id) = new {
            self.expected.push((change.path.clone(), id));
        }
        self.changes.retain(|c| c.path != change.path);
        self.changes.push(change);
        Ok(())
    }
    fn create_dir_all_inner(&mut self, target: &Path, mode: u32) -> io::Result<()> {
        if self.plan == "remove" || self.plan == "control-files" {
            return Err(invalid("exact file/removal plan cannot create directories"));
        }
        let target = self.target(target)?;
        if self.plan == "cache-entry" && target != self.root {
            return Err(invalid("cache directory creation cannot target a sidecar"));
        }
        self.artifacts_started = true;
        if mode & !0o777 != 0 || mode & 0o022 != 0 {
            return Err(invalid("unsafe output directory mode"));
        }
        self.checkpoint()?;
        let mut missing = Vec::new();
        let mut cursor = target.as_path();
        while !super::present(cursor)? {
            if !cursor.starts_with(&self.root) {
                return Err(invalid("missing directory outside mutation footprint"));
            }
            missing.push(cursor.to_path_buf());
            cursor = cursor.parent().ok_or_else(|| invalid("missing parent"))?;
        }
        Directory::open(cursor)?;
        for path in missing.into_iter().rev() {
            self.checkpoint()?;
            let parent = Directory::open(path.parent().ok_or_else(|| invalid("create parent"))?)?;
            parent.revalidate()?;
            if fingerprint(&path)?.is_some() {
                return Err(invalid("directory appeared before owned creation"));
            }
            self.journal(Some(&path), None)?;
            let intent = self.intent(
                super::catalog::EffectKind::Create,
                None,
                &path,
                None,
                None,
                None,
            )?;
            mkdir_at(
                &parent.file,
                path.file_name()
                    .ok_or_else(|| invalid("create component"))?,
                mode,
            )?;
            let created = Directory::open(&path)?;
            created.file.sync_all()?;
            parent.file.sync_all()?;
            self.changed(&path, None, Some(Identity::of(&created.file)?))?;
            self.complete_intent(&intent, Some(Identity::of(&created.file)?))?;
        }
        Ok(())
    }
    fn staging(&mut self, target: &Path) -> io::Result<(PathBuf, Directory, PathBuf, String)> {
        if self.plan == "remove" {
            return Err(invalid("Remove plan cannot stage payload"));
        }
        let target = self.target(target)?;
        if self.diagnostic_paths.contains(&target) {
            return Err(invalid("diagnostic inode is never staged"));
        }
        self.artifacts_started = true;
        self.checkpoint()?;
        let parent = Directory::open(target.parent().ok_or_else(|| invalid("staging parent"))?)?;
        // Parent permissions exclude uncooperative cross-user entry replacement.
        let metadata = parent.file.metadata()?;
        if metadata.uid() != uid() || metadata.mode() & 0o022 != 0 {
            return Err(invalid("publication parent is not owned/protected"));
        }
        let token = new_id();
        let stage = parent.path.join(format!(".iii-stage-{}-{token}", self.id));
        self.journal(Some(&stage), None)?;
        self.stages.push(token.clone());
        Ok((target, parent, stage, token))
    }
    fn stage_file_inner(&mut self, target: &Path, mode: u32) -> io::Result<StagedFile> {
        if self.plan == "cache-entry" {
            let mut name = self.root.as_os_str().to_os_string();
            name.push(".digest");
            if absolute(target)? != PathBuf::from(name) {
                return Err(invalid(
                    "cache-entry file staging is restricted to its digest sidecar",
                ));
            }
        }
        if mode & !0o777 != 0 || mode & 0o022 != 0 {
            return Err(invalid("unsafe staged file mode"));
        }
        let (target, parent, path, token) = self.staging(target)?;
        let intent = self.intent(
            super::catalog::EffectKind::Stage,
            None,
            &path,
            None,
            None,
            None,
        )?;
        let file = open_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("stage file"))?,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            mode,
        )?;
        let cache_writer_fence = self
            .lease
            .0
            .namespace
            .transaction()?
            .operation(&self.lease.0.id)?
            .cache
            .is_some();
        if cache_writer_fence {
            sentinel(&file)?;
        }
        let identity = Identity::of(&file)?;
        parent.file.sync_all()?;
        self.complete_intent(&intent, Some(identity.clone()))?;
        Ok(StagedFile {
            owner: self.id.clone(),
            operation: self.lease.0.id.clone(),
            namespace: self.lease.0.namespace.clone(),
            token,
            target,
            path,
            parent,
            file,
            identity,
            cache_writer_fence,
        })
    }
    fn stage_tree_inner(&mut self, target: &Path) -> io::Result<StagedTree> {
        if self.plan == "cache-entry" && absolute(target)? != self.root {
            return Err(invalid(
                "cache tree stage must target its exact source root",
            ));
        }
        if self.plan == "control-files" || self.plan == "erofs" {
            return Err(invalid("file plan cannot stage a tree"));
        }
        let (target, parent, path, token) = self.staging(target)?;
        let intent = self.intent(
            super::catalog::EffectKind::Stage,
            None,
            &path,
            None,
            None,
            None,
        )?;
        mkdir_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("stage tree"))?,
            0o700,
        )?;
        let directory = Directory::open(&path)?;
        let identity = Identity::of(&directory.file)?;
        parent.file.sync_all()?;
        self.complete_intent(&intent, Some(identity.clone()))?;
        Ok(StagedTree {
            owner: self.id.clone(),
            operation: self.lease.0.id.clone(),
            namespace: self.lease.0.namespace.clone(),
            token,
            target,
            path,
            parent,
            directory,
            identity,
        })
    }
    fn publish(
        &mut self,
        owner: &str,
        token: &str,
        parent: &Directory,
        stage: &Path,
        target: &Path,
        identity: &Identity,
    ) -> io::Result<()> {
        if owner != self.id || !self.stages.iter().any(|t| t == token) {
            return Err(invalid("stage belongs to another mutation"));
        }
        self.checkpoint()?;
        parent.revalidate()?;
        if !fingerprint(stage)?.is_some_and(|id| identity.same_inode(&id)) {
            return Err(invalid("staging identity replaced"));
        }
        let old = fingerprint(target)?;
        if old
            .as_ref()
            .is_some_and(|id| id.uid != uid() || id.mode & 0o022 != 0)
        {
            return Err(invalid("publication target is not owned/protected"));
        }
        // The old target must be exactly an originally pinned or core-created
        // identity; an arbitrary inode that appeared in the meantime is refused.
        let encoded = HostPath::new(target)?;
        let expected = self
            .expected
            .iter()
            .find(|(p, _)| p == &encoded)
            .map(|(_, id)| id.clone());
        if !same(&expected, &old) {
            return Err(invalid("unexpected target at publication"));
        }
        self.journal(
            None,
            Some(Change {
                path: encoded,
                old: old.clone(),
                new: Some(identity.clone()),
            }),
        )?;
        let backup = old.as_ref().map(|_| {
            parent
                .path
                .join(format!(".iii-prior-{}-{}", self.id, new_id()))
        });
        if let Some(backup) = &backup {
            self.journal(Some(backup), None)?;
        }
        let intent = self.intent(
            super::catalog::EffectKind::Publish,
            Some(stage),
            target,
            old.clone(),
            Some(identity.clone()),
            backup.as_deref(),
        )?;
        if let Some(backup) = &backup {
            no_replace(parent, target, parent, backup)?;
            parent.file.sync_all()?;
            if !same(&old, &fingerprint(backup)?) {
                return Err(invalid("old generation changed during preservation"));
            }
            self.changed(backup, None, old.clone())?;
        }
        #[cfg(test)]
        if self.publish_fault == Some("before-rename") {
            return Err(io::Error::other("injected publication boundary"));
        }
        no_replace(parent, stage, parent, target)?;
        #[cfg(test)]
        if self.publish_fault == Some("after-rename") {
            return Err(io::Error::other("injected publication boundary"));
        }
        parent.file.sync_all()?;
        if !fingerprint(target)?.is_some_and(|id| identity.same_inode(&id)) {
            return Err(invalid("published target replaced"));
        }
        self.changed(stage, Some(identity.clone()), None)?;
        self.changed(target, old, Some(identity.clone()))?;
        self.complete_intent(&intent, None)?;
        self.stages.retain(|t| t != token);
        Ok(())
    }
    fn remove_path_inner(&mut self, target: &Path) -> io::Result<()> {
        if self.plan != "remove" {
            return Err(invalid("remove_path requires Remove plan"));
        }
        let target = absolute(target)?;
        if !self.footprint.contains(&target) {
            return Err(invalid("removal requires an exact planned target"));
        }
        self.checkpoint()?;
        for component in target.components() {
            if let std::path::Component::Normal(n) = component {
                if n.to_str().is_some_and(|n| {
                    n.starts_with(".iii-stage-")
                        || n.starts_with(".iii-prior-")
                        || n.ends_with("-compat")
                        || n.starts_with("generation-")
                        || n.starts_with("backup")
                }) {
                    return Err(invalid("removal cannot target custody/evidence"));
                }
            }
        }
        let prior = fingerprint(&target)?;
        if prior.is_none() {
            if self
                .expected
                .iter()
                .any(|(p, _)| p.get().is_ok_and(|p| p == target))
            {
                return Err(invalid("removal target disappeared without intent"));
            }
            let intent = self.intent(
                super::catalog::EffectKind::Remove,
                None,
                &target,
                None,
                None,
                None,
            )?;
            self.complete_intent(&intent, None)?;
            return Ok(());
        }
        let mut inventory = Vec::new();
        let root_parent = Directory::open(
            target
                .parent()
                .ok_or_else(|| invalid("remove root parent"))?,
        )?;
        let expected_mount = mount_id(&root_parent.file)?;
        scan_remove(
            &target,
            prior.as_ref().ok_or_else(|| invalid("target missing"))?.dev,
            expected_mount,
            0,
            &mut inventory,
        )?;
        let mut links = std::collections::HashMap::new();
        for (_, id) in &inventory {
            if id.mode & libc::S_IFMT == libc::S_IFREG {
                *links.entry((id.dev, id.ino)).or_insert(0u64) += 1;
            }
        }
        if inventory.iter().any(|(_, id)| {
            id.mode & libc::S_IFMT == libc::S_IFREG
                && links.get(&(id.dev, id.ino)) != Some(&id.links)
        }) {
            return Err(invalid("removal has external hardlinks"));
        }
        self.artifacts_started = true;
        let top = self.intent(
            super::catalog::EffectKind::Remove,
            None,
            &target,
            prior,
            None,
            None,
        )?;
        for (path, expected) in inventory.into_iter().rev() {
            let parent = Directory::open(path.parent().ok_or_else(|| invalid("remove parent"))?)?;
            parent.revalidate()?;
            #[cfg(target_os = "linux")]
            let pinned = open_at(
                &parent.file,
                path.file_name().ok_or_else(|| invalid("remove name"))?,
                libc::O_PATH,
                0,
            )?;
            #[cfg(not(target_os = "linux"))]
            let pinned = open_at(
                &parent.file,
                path.file_name().ok_or_else(|| invalid("remove name"))?,
                libc::O_RDONLY | libc::O_NONBLOCK,
                0,
            )?;
            if mount_id(&pinned)? != expected_mount {
                return Err(invalid("removal mount changed"));
            }
            let actual = Identity::of(&pinned)?;
            if !actual.same_inode(&expected) {
                return Err(invalid("remove entry changed"));
            }
            let step = self.intent(
                super::catalog::EffectKind::Remove,
                None,
                &path,
                Some(expected.clone()),
                None,
                None,
            )?;
            let component = name(path.file_name().ok_or_else(|| invalid("remove name"))?)?;
            let flags = if expected.mode & libc::S_IFMT == libc::S_IFDIR {
                libc::AT_REMOVEDIR
            } else {
                0
            };
            syscall(unsafe { libc::unlinkat(parent.file.as_raw_fd(), component.as_ptr(), flags) })?;
            parent.file.sync_all()?;
            self.changed(&path, Some(expected), None)?;
            self.complete_intent(&step, None)?;
        }
        self.complete_intent(&top, None)
    }
    fn discard_file_inner(&mut self, stage: StagedFile) -> io::Result<()> {
        self.checkpoint()?;
        let stage = settle_cache_stage(stage)?;
        if !Arc::ptr_eq(&stage.namespace, &self.lease.0.namespace)
            || stage.operation != self.lease.0.id
            || stage.owner != self.id
            || !self.stages.iter().any(|s| s == &stage.token)
            || stage.path == stage.target
        {
            return Err(invalid(
                "discard requires this mutation's unpublished stage",
            ));
        }
        self.target(&stage.target)?;
        if self.spool.as_ref().is_some_and(|p| p != &stage.target) {
            return Err(invalid(
                "EROFS discard is restricted to its exact spool slot",
            ));
        }
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        if !op.entries.iter().any(|e| {
            e.role == PathRole::MutableFile && e.path.get().is_ok_and(|p| p == stage.target)
        }) {
            return Err(invalid(
                "discard slot requires exact MutableFile declaration",
            ));
        }
        stage.parent.revalidate()?;
        let file_identity = Identity::of(&stage.file)?;
        if !stage.identity.same_inode(&file_identity)
            || file_identity.uid != uid()
            || file_identity.links != 1
            || file_identity.mode & libc::S_IFMT != libc::S_IFREG
            || !fingerprint(&stage.path)?
                .is_some_and(|id| stage.identity.same_inode(&id) && id.links == 1)
        {
            return Err(invalid("discard stage identity replaced"));
        }
        let parent = Identity::of(&stage.parent.file)?;
        if parent.uid != uid() || parent.mode & 0o022 != 0 {
            return Err(invalid("discard parent is not protected"));
        }
        self.discard_boundary("before-intent")?;
        let journal = op
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("discard journal missing"))?;
        if journal.disposals.len() >= 256
            || journal.disposals.iter().any(|d| d.stage == stage.token)
        {
            return Err(limit());
        }
        journal.disposals.push(super::catalog::Disposal {
            stage: stage.token.clone(),
            slot: HostPath::new(&stage.target)?,
            path: HostPath::new(&stage.path)?,
            parent,
            file: file_identity,
            discarded: false,
        });
        tx.save()?;
        drop(tx);
        self.discard_boundary("after-intent")?;
        stage.parent.revalidate()?;
        if !fingerprint(&stage.path)?
            .is_some_and(|id| stage.identity.same_inode(&id) && id.links == 1)
        {
            return Err(invalid("discard stage changed after intent"));
        }
        unlink_at(
            &stage.parent.file,
            stage
                .path
                .file_name()
                .ok_or_else(|| invalid("discard stage name"))?,
        )?;
        self.discard_boundary("after-unlink")?;
        stage.parent.file.sync_all()?;
        self.discard_boundary("after-sync")?;
        let mut tx = self.lease.0.namespace.transaction()?;
        let journal = tx
            .operation_mut(&self.lease.0.id)?
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("discard journal missing"))?;
        let disposal = journal
            .disposals
            .iter_mut()
            .find(|d| d.stage == stage.token)
            .ok_or_else(|| invalid("discard intent missing"))?;
        disposal.discarded = true;
        tx.save()?;
        drop(tx);
        self.discard_boundary("after-completion")?;
        self.changed(&stage.path, Some(stage.identity.clone()), None)?;
        self.stages.retain(|s| s != &stage.token);
        Ok(())
    }
    fn discard_boundary(&self, boundary: &str) -> io::Result<()> {
        #[cfg(test)]
        if self.discard_fault == Some(boundary) {
            return Err(io::Error::other("injected discard boundary fault"));
        }
        let _ = boundary;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn fault_publish_at(&mut self, boundary: &'static str) {
        self.publish_fault = Some(boundary);
    }
    #[cfg(test)]
    pub(super) fn fault_discard_at(&mut self, boundary: &'static str) {
        self.discard_fault = Some(boundary);
    }
    fn publish_file_inner(&mut self, stage: StagedFile) -> io::Result<()> {
        if let Some(spool) = &self.spool {
            if stage.target != self.root {
                return Err(invalid("EROFS spool must be discarded, never published"));
            }
            let tx = self.lease.0.namespace.transaction()?;
            let journal = tx
                .operation(&self.lease.0.id)?
                .journals
                .iter()
                .find(|j| j.id == self.id)
                .ok_or_else(|| invalid("missing journal"))?;
            if !journal
                .disposals
                .iter()
                .any(|d| d.discarded && d.slot.get().is_ok_and(|p| &p == spool))
            {
                return Err(invalid(
                    "EROFS output requires completed spool disposal first",
                ));
            }
            if fingerprint(spool)?
                .as_ref()
                .is_some_and(|id| id.uid != uid())
            {
                return Err(invalid("spool target ownership changed"));
            }
        }
        if !Arc::ptr_eq(&stage.namespace, &self.lease.0.namespace)
            || stage.operation != self.lease.0.id
        {
            return Err(invalid("stage context/operation mismatch"));
        }
        if !stage.identity.same_inode(&Identity::of(&stage.file)?)
            || stage.file.metadata()?.nlink() != 1
        {
            return Err(invalid("staged file changed"));
        }
        stage.file.sync_all()?;
        let stage = settle_cache_stage(stage)?;
        self.publish(
            &stage.owner,
            &stage.token,
            &stage.parent,
            &stage.path,
            &stage.target,
            &stage.identity,
        )
    }
    fn publish_tree_inner(&mut self, stage: StagedTree) -> io::Result<()> {
        if !Arc::ptr_eq(&stage.namespace, &self.lease.0.namespace)
            || stage.operation != self.lease.0.id
        {
            return Err(invalid("stage context/operation mismatch"));
        }
        stage.directory.revalidate()?;
        let mut count = 0;
        sync_tree(&stage.directory, &mut count, 0)?;
        // Capture the exact identities belonging to this owned staged tree.
        // Only these observed versions can replace admitted descendant pins.
        let mut updates = Vec::new();
        let tx = self.lease.0.namespace.transaction()?;
        for entry in &tx.operation(&self.lease.0.id)?.entries {
            if entry.role == PathRole::Strict {
                continue;
            }
            for (path, old) in &entry.pins {
                let path = path.get()?;
                if path != stage.target
                    && path.starts_with(&stage.target)
                    && !updates.iter().any(|(p, _, _)| p == &path)
                {
                    let tail = path.strip_prefix(&stage.target).map_err(io::Error::other)?;
                    updates.push((
                        path.clone(),
                        Some(old.clone()),
                        fingerprint(&stage.path.join(tail))?,
                    ));
                }
            }
        }
        drop(tx);
        self.publish(
            &stage.owner,
            &stage.token,
            &stage.parent,
            &stage.path,
            &stage.target,
            &stage.identity,
        )?;
        for (path, old, new) in updates {
            self.changed(&path, old, new)?;
        }
        Ok(())
    }
    fn rename_preserving_inner(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if self.plan != "adopt" {
            return Err(invalid("rename_preserving requires Adopt plan"));
        }
        let (from, to) = (self.target(from)?, self.target(to)?);
        if self
            .diagnostic_paths
            .iter()
            .any(|p| p.starts_with(&from) || p.starts_with(&to))
        {
            return Err(invalid("diagnostic inode cannot be renamed"));
        }
        self.artifacts_started = true;
        if overlaps(&from, &to) {
            return Err(invalid("overlapping adopt paths"));
        }
        self.checkpoint()?;
        let old = fingerprint(&from)?.ok_or_else(|| invalid("adopt source missing"))?;
        if old.uid != uid() || old.mode & 0o022 != 0 {
            return Err(invalid("adopt source is not owned/protected"));
        }
        let encoded = HostPath::new(&from)?;
        if !self
            .expected
            .iter()
            .any(|(p, id)| p == &encoded && id.same_inode(&old))
        {
            return Err(invalid("adopt source was not pinned before mutation"));
        }
        if fingerprint(&to)?.is_some() {
            return Err(invalid("adopt target already exists"));
        }
        let source = Directory::open(from.parent().ok_or_else(|| invalid("source parent"))?)?;
        let target = Directory::open(to.parent().ok_or_else(|| invalid("target parent"))?)?;
        self.journal(
            None,
            Some(Change {
                path: HostPath::new(&from)?,
                old: Some(old.clone()),
                new: None,
            }),
        )?;
        self.journal(
            None,
            Some(Change {
                path: HostPath::new(&to)?,
                old: None,
                new: Some(old.clone()),
            }),
        )?;
        let intent = self.intent(
            super::catalog::EffectKind::Adopt,
            Some(&from),
            &to,
            None,
            Some(old.clone()),
            None,
        )?;
        no_replace(&source, &from, &target, &to)?;
        source.file.sync_all()?;
        target.file.sync_all()?;
        if !fingerprint(&to)?.is_some_and(|id| old.same_inode(&id)) {
            return Err(invalid("adopt identity changed"));
        }
        let descendants: Vec<_> = self
            .expected
            .iter()
            .filter_map(|(p, id)| {
                p.get()
                    .ok()
                    .filter(|p| p != &from && p.starts_with(&from))
                    .map(|p| (p, id.clone()))
            })
            .collect();
        for (path, id) in descendants {
            let destination = to.join(path.strip_prefix(&from).map_err(io::Error::other)?);
            self.changed(&path, Some(id.clone()), None)?;
            self.changed(&destination, None, Some(id))?;
        }
        self.changed(&from, Some(old.clone()), None)?;
        self.changed(&to, None, Some(old))?;
        self.complete_intent(&intent, None)?;
        Ok(())
    }
    pub(crate) fn commit(mut self) -> io::Result<EffectReceipt> {
        if self.diagnostics.handles.load(Ordering::Acquire) != 0 {
            return Err(invalid("diagnostic handles still outstanding"));
        }
        if !self.stages.is_empty() {
            return Err(invalid("unpublished stages remain"));
        }
        self.checkpoint()?;
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        if op.uncertain {
            return Err(invalid("uncertain publication"));
        }
        // Only exact old/new transitions issued above can change recorded pins.
        // Other entries and strict identities are never refreshed from the path.
        for entry in &mut op.entries {
            if entry.role == PathRole::Strict {
                continue;
            }
            for change in &self.changes {
                entry.pins.retain(|(p, _)| p != &change.path);
                let changed = change.path.get()?;
                if entry.path.get()?.starts_with(&changed) {
                    if let Some(new) = &change.new {
                        entry.pins.push((change.path.clone(), new.clone()));
                    }
                }
            }
        }
        let journal = op
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("missing journal"))?;
        if journal.intents.iter().any(|i| !i.completed && !i.cancelled) {
            return Err(invalid("unresolved before-effect intent"));
        }
        if journal.disposals.iter().any(|d| !d.discarded) {
            return Err(invalid("unresolved stage discard"));
        }
        let disposals = journal.disposals.clone();
        journal.committed = true;
        op.holds.retain(|h| h.id != self.id);
        tx.save()?;
        self.committed = true;
        Ok(EffectReceipt {
            _operation: self.lease.0.id.clone(),
            _journal: self.id.clone(),
            _epoch: self.epoch,
            _changes: self.changes.clone(),
            _disposals: disposals,
        })
    }
}
impl Drop for MutationGuard {
    fn drop(&mut self) {
        if !self.committed {
            // No artifact effect and no surviving diagnostic writer: an ordinary
            // refused lock acquisition is abortable, not a successful receipt.
            if !self.artifacts_started
                && self.diagnostics.handles.load(Ordering::Acquire) == 0
                && !self.diagnostics.failed.load(Ordering::Acquire)
            {
                if let Ok(mut tx) = self.lease.0.namespace.transaction() {
                    if let Ok(op) = tx.operation_mut(&self.lease.0.id) {
                        op.holds.retain(|h| h.id != self.id);
                        op.journals.retain(|j| j.id != self.id);
                        for entry in &mut op.entries {
                            if entry.role == PathRole::Strict {
                                continue;
                            }
                            for change in &self.changes {
                                if let Ok(path) = change.path.get() {
                                    if entry.path.get().is_ok_and(|p| p.starts_with(&path)) {
                                        entry.pins.retain(|(p, _)| p != &change.path);
                                        if let Some(new) = &change.new {
                                            entry.pins.push((change.path.clone(), new.clone()));
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx.save();
                    }
                }
            } else {
                let _ = self.lease.finish_hold(&self.id, true);
            }
        }
    }
}
impl MutationGuard {
    fn terminal<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.diagnostics.failed.store(true, Ordering::Release);
            let _ = self.lease.finish_hold(&self.id, true);
        }
        result
    }
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        self.terminal(self.checkpoint_inner())
    }
    pub(crate) fn create_dir_all(&mut self, target: &Path, mode: u32) -> io::Result<()> {
        let result = self.create_dir_all_inner(target, mode);
        self.terminal(result)
    }
    pub(crate) fn stage_file(&mut self, target: &Path, mode: u32) -> io::Result<StagedFile> {
        let result = self.stage_file_inner(target, mode);
        self.terminal(result)
    }
    pub(crate) fn stage_tree(&mut self, target: &Path) -> io::Result<StagedTree> {
        let result = self.stage_tree_inner(target);
        self.terminal(result)
    }
    pub(crate) fn remove_path(&mut self, target: &Path) -> io::Result<()> {
        let result = self.remove_path_inner(target);
        self.terminal(result)
    }
    pub(crate) fn discard_file(&mut self, stage: StagedFile) -> io::Result<()> {
        let result = self.discard_file_inner(stage);
        self.terminal(result)
    }
    pub(crate) fn publish_file(&mut self, stage: StagedFile) -> io::Result<()> {
        let result = self.publish_file_inner(stage);
        self.terminal(result)
    }
    pub(crate) fn publish_tree(&mut self, stage: StagedTree) -> io::Result<()> {
        let result = self.publish_tree_inner(stage);
        self.terminal(result)
    }
    pub(crate) fn rename_preserving(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let result = self.rename_preserving_inner(from, to);
        self.terminal(result)
    }
    pub(crate) fn diagnostic_lock(&mut self, path: &Path) -> io::Result<DiagnosticLock> {
        let result = self.diagnostic_lock_inner(path);
        self.terminal(result)
    }
}
struct DiagnosticState {
    handles: AtomicUsize,
    failed: AtomicBool,
}
pub(crate) struct DiagnosticLock {
    lease: Lease,
    mutation: String,
    file: File,
    parent: Directory,
    path: PathBuf,
    identity: Identity,
    owned: bool,
    state: Arc<DiagnosticState>,
}
impl AsFd for DiagnosticLock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}
impl MutationGuard {
    fn diagnostic_lock_inner(&mut self, path: &Path) -> io::Result<DiagnosticLock> {
        self.checkpoint()?;
        let path = absolute(path)?;
        if self.diagnostic_paths.contains(&path) {
            return Err(busy());
        }
        let mut sidecar = self.root.as_os_str().to_os_string();
        sidecar.push(".lock");
        if self.plan == "erofs"
            && (self.spool.as_ref().is_some_and(|p| overlaps(p, &path))
                || overlaps(&self.root, &path))
        {
            return Err(invalid("diagnostic aliases an EROFS artifact"));
        }
        let related = match self.plan.as_str() {
            "cache-entry" => {
                let mut name = self.root.as_os_str().to_os_string();
                name.push(".lock");
                path == PathBuf::from(name)
            }
            "erofs" => path == PathBuf::from(sidecar),
            "cache" | "image-pair" => path.starts_with(&self.root) && path != self.root,
            _ => false,
        };
        if !related {
            return Err(invalid(
                "diagnostic path is not related to the mutation plan",
            ));
        }
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        let declared = op
            .entries
            .iter()
            .find(|e| e.role == PathRole::MutableFile && e.path.get().is_ok_and(|p| p == path))
            .ok_or_else(|| invalid("diagnostic requires exact MutableFile declaration"))?;
        super::validate_entry(declared)?;
        if self.plan != "erofs" {
            let component = path.file_name().ok_or_else(|| invalid("diagnostic name"))?;
            use std::os::unix::ffi::{OsStrExt, OsStringExt};
            let protected_name = component
                .as_bytes()
                .strip_suffix(b".lock")
                .filter(|n| !n.is_empty())
                .ok_or_else(|| invalid("diagnostic must name an exact lock sidecar"))?;
            let protected =
                path.with_file_name(std::ffi::OsString::from_vec(protected_name.to_vec()));
            if !op.entries.iter().any(|e| {
                e.path.get().is_ok_and(|p| p == protected)
                    && matches!(e.role, PathRole::MutableTree | PathRole::MutableFile)
            }) {
                return Err(invalid(
                    "diagnostic has no explicitly declared protected artifact",
                ));
            }
        }
        let expected_identity = declared
            .pins
            .iter()
            .find(|(p, _)| p.get().is_ok_and(|p| p == path))
            .map(|(_, id)| id.clone());
        if op
            .holds
            .iter()
            .any(|h| h.id != self.id && h.path.get().is_ok_and(|p| overlaps(&p, &path)))
        {
            return Err(busy());
        }
        if op.holds.len() >= 256 {
            return Err(limit());
        }
        op.holds.push(super::catalog::Hold {
            id: self.id.clone(),
            path: HostPath::new(&path)?,
            kind: HoldKind::Mutation,
            epoch: self.epoch,
            owner: Some(super::catalog::ResourceOwner {
                holder: self.lease.0.holder.clone(),
                runtime: None,
            }),
        });
        tx.save()?;
        drop(tx);
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("diagnostic parent"))?)?;
        let metadata = parent.file.metadata()?;
        if metadata.uid() != uid() || metadata.mode() & 0o022 != 0 {
            return Err(invalid("unsafe diagnostic parent"));
        }
        self.journal(Some(&path), None)?;
        let intent = self.intent(
            super::catalog::EffectKind::Diagnostic,
            None,
            &path,
            expected_identity.clone(),
            expected_identity.clone(),
            None,
        )?;
        let component = path.file_name().ok_or_else(|| invalid("diagnostic name"))?;
        let mut created = false;
        let file = match open_at(
            &parent.file,
            component,
            libc::O_RDWR | libc::O_NONBLOCK | libc::O_CREAT | libc::O_EXCL,
            0o644,
        ) {
            Ok(file) => {
                created = true;
                file
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                open_at(&parent.file, component, libc::O_RDWR | libc::O_NONBLOCK, 0)?
            }
            Err(e) => return Err(e),
        };
        let identity = Identity::of(&file)?;
        if expected_identity
            .as_ref()
            .is_some_and(|old| !old.same_inode(&identity))
        {
            return Err(invalid("diagnostic inode changed since admission"));
        }
        if identity.uid != uid()
            || identity.mode & libc::S_IFMT != libc::S_IFREG
            || identity.mode & 0o022 != 0
            || identity.links != 1
        {
            return Err(invalid("unsafe diagnostic file"));
        }
        parent.revalidate()?;
        if !fingerprint(&path)?.is_some_and(|id| identity.same_inode(&id)) {
            return Err(invalid("diagnostic changed during open"));
        }
        if created {
            file.sync_all()?;
            parent.file.sync_all()?;
        }
        if expected_identity.is_none() {
            self.changed(&path, None, Some(identity.clone()))?;
        }
        self.complete_intent(&intent, Some(identity.clone()))?;
        self.diagnostic_paths.push(path.clone());
        self.diagnostics.handles.fetch_add(1, Ordering::AcqRel);
        Ok(DiagnosticLock {
            lease: self.lease.clone(),
            mutation: self.id.clone(),
            file,
            parent,
            path,
            identity,
            owned: false,
            state: self.diagnostics.clone(),
        })
    }
}
impl DiagnosticLock {
    fn checked<T>(&self, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        if self.state.failed.load(Ordering::Acquire) {
            return Err(invalid("diagnostic failure is terminal"));
        }
        let result = action();
        if result.is_err() {
            self.state.failed.store(true, Ordering::Release);
            let _ = self.lease.finish_hold(&self.mutation, true);
        }
        result
    }
    pub(crate) fn revalidate(&self) -> io::Result<()> {
        self.checked(|| {
            self.parent.revalidate()?;
            if !self.identity.same_inode(&Identity::of(&self.file)?)
                || !fingerprint(&self.path)?
                    .is_some_and(|id| self.identity.same_inode(&id) && id.links == 1)
            {
                return Err(invalid("diagnostic inode replaced"));
            }
            let tx = self.lease.0.namespace.transaction()?;
            self.lease.validate_continuation(&tx)?;
            let op = tx.operation(&self.lease.0.id)?;
            if op.closed || op.uncertain || !op.holds.iter().any(|h| h.id == self.mutation) {
                return Err(invalid("diagnostic mutation no longer active"));
            }
            Ok(())
        })
    }
    pub(crate) fn try_lock_exclusive(&mut self) -> io::Result<bool> {
        self.revalidate()?;
        if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(false);
            }
            return self.checked(|| Err(error));
        }
        self.owned = true;
        self.revalidate()?;
        Ok(true)
    }
    pub(crate) fn lock_exclusive(&mut self) -> io::Result<()> {
        self.revalidate()?;
        // All catalog transactions above are dropped BEFORE this blocking call.
        if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_EX) } < 0 {
            let e = io::Error::last_os_error();
            return self.checked(|| Err(e));
        }
        self.owned = true;
        self.revalidate()
    }
    pub(crate) fn write_pid(&mut self, pid: u32) -> io::Result<()> {
        self.revalidate()?;
        self.checked(|| {
            if !self.owned {
                return Err(invalid("diagnostic write requires this handle's flock"));
            }
            let bytes = format!("{pid}\n");
            self.file.write_all_at(bytes.as_bytes(), 0)?;
            self.file.set_len(bytes.len() as u64)?;
            Ok(())
        })
    }
    pub(crate) fn clear(&mut self) -> io::Result<()> {
        self.revalidate()?;
        self.checked(|| {
            if !self.owned {
                return Err(invalid("diagnostic clear requires this handle's flock"));
            }
            self.file.set_len(0)
        })
    }
}
impl Drop for DiagnosticLock {
    fn drop(&mut self) {
        self.state.handles.fetch_sub(1, Ordering::AcqRel);
    }
}
fn scan_remove(
    path: &Path,
    device: u64,
    expected_mount: u64,
    depth: usize,
    result: &mut Vec<(PathBuf, Identity)>,
) -> io::Result<()> {
    if depth > 64 || result.len() >= 256 {
        return Err(limit());
    }
    let parent = Directory::open(path.parent().ok_or_else(|| invalid("remove parent"))?)?;
    #[cfg(target_os = "linux")]
    let pinned = open_at(
        &parent.file,
        path.file_name().ok_or_else(|| invalid("remove name"))?,
        libc::O_PATH,
        0,
    )?;
    #[cfg(not(target_os = "linux"))]
    let pinned = open_at(
        &parent.file,
        path.file_name().ok_or_else(|| invalid("remove name"))?,
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    )?;
    if mount_id(&pinned)? != expected_mount {
        return Err(invalid("removal crosses a mount"));
    }
    let metadata = pinned.metadata()?;
    let identity = Identity::metadata(&metadata);
    if identity.uid != uid()
        || identity.dev != device
        || (!metadata.is_file() && !metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err(invalid("unsafe removal entry/mount"));
    }
    if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n.starts_with(".iii-stage-")
            || n.starts_with(".iii-prior-")
            || n.starts_with("generation-")
            || n.starts_with("backup")
            || n.ends_with("-compat")
    }) {
        return Err(invalid("removal protects migration evidence"));
    }
    result.push((path.to_path_buf(), identity));
    if metadata.is_dir() {
        let directory = Directory::open(path)?;
        let mut children = Vec::new();
        for child in std::fs::read_dir(path)? {
            if children.len() >= 256 {
                return Err(limit());
            }
            children.push(child?);
        }
        children.sort_by_key(|e| e.file_name());
        for child in children {
            directory.revalidate()?;
            scan_remove(&child.path(), device, expected_mount, depth + 1, result)?;
        }
    }
    Ok(())
}
fn sync_tree(directory: &Directory, count: &mut usize, depth: usize) -> io::Result<()> {
    if depth > 64 {
        return Err(limit());
    }
    directory.revalidate()?;
    for entry in std::fs::read_dir(&directory.path)? {
        *count += 1;
        if *count > 4096 {
            return Err(limit());
        }
        let entry = entry?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.uid() != uid() || (!meta.file_type().is_symlink() && meta.mode() & 0o022 != 0) {
            return Err(invalid("unsafe staged tree ownership/mode"));
        }
        if meta.is_dir() {
            sync_tree(&Directory::open(&path)?, count, depth + 1)?;
        } else if meta.is_file() && meta.nlink() == 1 {
            open_at(&directory.file, &entry.file_name(), libc::O_RDONLY, 0)?.sync_all()?;
        } else if meta.file_type().is_symlink() { /* retained symlink data; never follow its target */
        } else {
            return Err(invalid("unsupported staged tree member"));
        }
    }
    directory.revalidate()?;
    directory.file.sync_all()
}
