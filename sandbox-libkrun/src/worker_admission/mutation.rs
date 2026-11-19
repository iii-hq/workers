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
        let (root, label, role) = match plan {
            MutationPlan::Create { root } => (root, "create", PathRole::MutableTree),
            MutationPlan::Adopt { root } => (root, "adopt", PathRole::MutableTree),
            MutationPlan::Reclone { root } => (root, "reclone", PathRole::MutableTree),
            MutationPlan::Cache { root } => (root, "cache", PathRole::MutableTree),
            MutationPlan::Erofs { output } => (output, "erofs", PathRole::MutableFile),
            MutationPlan::ImagePair { directory } => {
                (directory, "image-pair", PathRole::MutableTree)
            }
        };
        let root = absolute(&root)?;
        if let Some(identity) = fingerprint(&root)? {
            if identity.uid != uid() || identity.mode & 0o022 != 0 {
                return Err(invalid("mutable root is not owned/protected"));
            }
        }
        let id = lease.hold(&root, HoldKind::Mutation, Some(role))?;
        // From this point any failure preserves the hold as uncertain evidence.
        let result = (|| {
            let mut tx = lease.0.namespace.transaction()?;
            let op = tx.operation_mut(&lease.0.id)?;
            if op.journals.len() >= 256 {
                return Err(limit());
            }
            op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
            let epoch = op.epoch;
            let mut expected = Vec::new();
            for e in &op.entries {
                for pin in &e.pins {
                    if overlaps(&pin.0.get()?, &root) && !expected.iter().any(|(p, _)| p == &pin.0)
                    {
                        expected.push(pin.clone());
                    }
                }
            }
            op.journals.push(Journal {
                id: id.clone(),
                plan: label.into(),
                root: HostPath::new(&root)?,
                epoch,
                committed: false,
                changes: Vec::new(),
                artifacts: Vec::new(),
                disposals: Vec::new(),
            });
            tx.save()?;
            Ok((epoch, expected))
        })();
        match result {
            Ok((epoch, expected)) => Ok(Self {
                lease,
                id,
                root,
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
            }),
            Err(e) => {
                let _ = lease.finish_hold(&id, true);
                Err(e)
            }
        }
    }
    fn target(&self, path: &Path) -> io::Result<PathBuf> {
        let path = absolute(path)?;
        if !(path == self.root || (self.plan != "erofs" && path.starts_with(&self.root))) {
            return Err(invalid("mutation exceeds declared footprint"));
        }
        Ok(path)
    }
    fn checkpoint_inner(&self) -> io::Result<()> {
        if self.diagnostics.failed.load(Ordering::Acquire) {
            return Err(invalid("diagnostic checkpoint is terminal"));
        }
        let tx = self.lease.0.namespace.transaction()?;
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
        let target = self.target(target)?;
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
        }
        Ok(())
    }
    fn staging(&mut self, target: &Path) -> io::Result<(PathBuf, Directory, PathBuf, String)> {
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
        if mode & !0o777 != 0 || mode & 0o022 != 0 {
            return Err(invalid("unsafe staged file mode"));
        }
        let (target, parent, path, token) = self.staging(target)?;
        let file = open_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("stage file"))?,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            mode,
        )?;
        let identity = Identity::of(&file)?;
        parent.file.sync_all()?;
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
        })
    }
    fn stage_tree_inner(&mut self, target: &Path) -> io::Result<StagedTree> {
        let (target, parent, path, token) = self.staging(target)?;
        mkdir_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("stage tree"))?,
            0o700,
        )?;
        let directory = Directory::open(&path)?;
        let identity = Identity::of(&directory.file)?;
        parent.file.sync_all()?;
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
        if old.is_some() {
            let backup = parent
                .path
                .join(format!(".iii-prior-{}-{}", self.id, new_id()));
            self.journal(Some(&backup), None)?;
            no_replace(parent, target, parent, &backup)?;
            parent.file.sync_all()?;
            if !same(&old, &fingerprint(&backup)?) {
                return Err(invalid("old generation changed during preservation"));
            }
            self.changed(&backup, None, old.clone())?;
            // No automatic deletion, rollback or OLD reimport, including errors.
        }
        no_replace(parent, stage, parent, target)?;
        parent.file.sync_all()?;
        if !fingerprint(target)?.is_some_and(|id| identity.same_inode(&id)) {
            return Err(invalid("published target replaced"));
        }
        self.changed(stage, Some(identity.clone()), None)?;
        self.changed(target, old, Some(identity.clone()))?;
        self.stages.retain(|t| t != token);
        Ok(())
    }
    fn discard_file_inner(&mut self, stage: StagedFile) -> io::Result<()> {
        self.checkpoint()?;
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
    pub(super) fn fault_discard_at(&mut self, boundary: &'static str) {
        self.discard_fault = Some(boundary);
    }
    fn publish_file_inner(&mut self, stage: StagedFile) -> io::Result<()> {
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
        let mut sidecar = self.root.as_os_str().to_os_string();
        sidecar.push(".lock");
        let related = match self.plan.as_str() {
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
        });
        tx.save()?;
        drop(tx);
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("diagnostic parent"))?)?;
        let metadata = parent.file.metadata()?;
        if metadata.uid() != uid() || metadata.mode() & 0o022 != 0 {
            return Err(invalid("unsafe diagnostic parent"));
        }
        self.journal(Some(&path), None)?;
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
            let op = tx.operation(&self.lease.0.id)?;
            if op.uncertain || !op.holds.iter().any(|h| h.id == self.mutation) {
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
