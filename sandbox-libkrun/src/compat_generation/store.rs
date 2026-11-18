// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::{
    Environment, Error, FORMAT, Faults, Layout, OfflineGrant, Provision, Record, Result, Selector,
    Step, decode, encode, fs, tree,
};
use serde::{Deserialize, Serialize};
use std::fs::File;

/// No callable endpoint or production constructor issues OfflineGrant. These
/// operations require that unforgeable in-module grant AND held offline leases.
/// Store holds a lock on custody outside removable VM/build/generation trees.
pub(super) struct Store<'a> {
    grant: &'a OfflineGrant,
    custody: fs::Dir,
    backups: fs::Dir,
    _custody_lease: File,
    _backup_lease: File,
    protected: Vec<fs::Dir>,
    // Synchronous transaction handle cannot be shared across threads. Separate
    // Store instances contend on the directory flock. No callback/reentrancy.
    _single_thread: std::marker::PhantomData<std::cell::Cell<()>>,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Committed(u64),
    AlreadyCommitted(u64),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    version: u32,
    binding: String,
    operation: String,
    #[serde(deserialize_with = "super::object")]
    environment: Environment,
    snapshot_digest: String,
}

impl<'a> Store<'a> {
    /// Roots are existing private owner directories bound by local configuration.
    /// Every managed VM/build/source/share root must be passed in protected; a
    /// backup inside any of them (or containing one) refuses before mutation.
    pub(super) fn open(
        grant: &'a OfflineGrant,
        custody: fs::Dir,
        backups: fs::Dir,
        protected: &[&fs::Dir],
    ) -> Result<Self> {
        grant.environment.validate()?;
        if !super::sha(&grant.binding) || protected.is_empty() {
            return Err(Error::Custody);
        }
        custody.revalidate(true)?;
        backups.revalidate(true)?;
        for a in [&custody, &backups] {
            if !a.disjoint(&grant.source)? {
                return Err(Error::Custody);
            }
            for root in protected {
                if !a.disjoint(root)? {
                    return Err(Error::Custody);
                }
            }
        }
        if !custody.disjoint(&backups)? {
            return Err(Error::Custody);
        }
        let custody_lease = custody.lock()?;
        let backup_lease = backups.lock()?;
        if !custody.exists("active.json")? {
            grant.source.revalidate(true)?;
            grant.pinned.unchanged(&grant.source)?;
            validate_payload(&grant.pinned, &grant.environment)?;
        }
        Ok(Self {
            grant,
            custody,
            backups,
            _custody_lease: custody_lease,
            _backup_lease: backup_lease,
            protected: protected
                .iter()
                .map(|root| root.duplicate())
                .collect::<Result<Vec<_>>>()?,
            _single_thread: std::marker::PhantomData,
        })
    }
    fn validate_roots(&self) -> Result<()> {
        self.custody.revalidate(true)?;
        self.backups.revalidate(true)?;
        for root in &self.protected {
            root.revalidate(false)?;
            if !self.custody.disjoint(root)? || !self.backups.disjoint(root)? {
                return Err(Error::Custody);
            }
        }
        Ok(())
    }
    fn selector(&self) -> Result<Option<Selector>> {
        if !self.custody.exists("active.json")? {
            return Ok(None);
        }
        let s: Selector = decode(&self.custody.read_record("active.json")?)?;
        if s.version != FORMAT
            || s.binding != self.grant.binding
            || !super::id(&s.operation)
            || s.generation == 0
            || !super::sha(&s.record_sha256)
        {
            return Err(Error::Corrupt);
        }
        Ok(Some(s))
    }
    fn record(&self, operation: &str) -> Result<(Record, fs::Dir, String)> {
        if !super::id(operation) {
            return Err(Error::Corrupt);
        }
        let generation = self.custody.child(&format!("generation-{operation}"))?;
        generation.revalidate(true)?;
        let bytes = generation.read_record("receipt.json")?;
        let record: Record = decode(&bytes)?;
        record.validate(&self.grant.binding)?;
        if record.operation != operation {
            return Err(Error::Corrupt);
        }
        let data = generation.child("data")?;
        data.revalidate(true)?;
        Ok((record, data, fs::digest(&bytes)))
    }
    /// Select CURRENT generation, never original or an archived overlay. The
    /// source snapshot hash is not compared with mutable application bytes.
    /// Immutable lower identity IS checked every time before use.
    pub(super) fn selected(&self) -> Result<Option<Selected<'_>>> {
        self.validate_roots()?;
        let Some(s) = self.selector()? else {
            if !fs::names(self.custody.fd())?.is_empty() {
                return Err(Error::Interrupted);
            }
            return Ok(None);
        };
        let (r, data, digest) = self.record(&s.operation)?;
        if r.generation != s.generation
            || digest != s.record_sha256
            || r.environment != self.grant.environment
        {
            return Err(Error::Corrupt);
        }
        let now = tree::Inventory::scan(&data)?;
        validate_payload(&now, &r.environment)?;
        Ok(Some(Selected {
            record: r,
            data,
            _lease: &self._custody_lease,
        }))
    }
    fn expected(&self, operation: &str, expected: u64) -> Result<Option<Outcome>> {
        if !super::id(operation) {
            return Err(Error::Corrupt);
        }
        // Recovery may inspect a complete unselected generation. Ordinary
        // selection refuses nonempty custody without a selector; never boot it.
        let current = if self.selector()?.is_some() {
            self.selected()?
        } else {
            None
        };
        if let Some(selected) = &current {
            if selected.record.operation == operation {
                return Ok(Some(Outcome::AlreadyCommitted(selected.record.generation)));
            }
        }
        if current.as_ref().map_or(0, |s| s.record.generation) != expected {
            return Err(Error::Conflict);
        }
        Ok(None)
    }
    /// Initial import only. Once any active generation exists this NEVER reads
    /// old data again; retry of the same committed operation is a no-op.
    pub(super) fn capture(
        &self,
        operation: &str,
        expected: u64,
        fault: &mut Faults,
    ) -> Result<Outcome> {
        if let Some(done) = self.expected(operation, expected)? {
            self.custody.sync(fault)?;
            return Ok(done);
        }
        if expected != 0 {
            return Err(Error::Conflict);
        }
        self.grant.source.revalidate(true)?;
        self.grant.pinned.unchanged(&self.grant.source)?;
        self.stage(operation, 0, &self.grant.source, &self.grant.pinned, fault)
    }
    /// Compatible rebuild's OFFLINE snapshot step. It clones CURRENT bytes,
    /// including NEW writes. Hooks/provisioning are not run by this component.
    /// A different environment/lower is never silently accepted.
    pub(super) fn rebuild(
        &self,
        operation: &str,
        expected: u64,
        environment: &Environment,
        fault: &mut Faults,
    ) -> Result<Outcome> {
        if environment != &self.grant.environment {
            return Err(Error::Unsupported(
                "different application/OS/profile requires separate migration",
            ));
        }
        if let Some(done) = self.expected(operation, expected)? {
            self.custody.sync(fault)?;
            return Ok(done);
        }
        let current = self.selected()?.ok_or(Error::Conflict)?;
        let snapshot = tree::Inventory::scan(&current.data)?;
        self.stage(operation, expected, &current.data, &snapshot, fault)
    }
    fn stage(
        &self,
        operation: &str,
        parent: u64,
        source: &fs::Dir,
        snapshot: &tree::Inventory,
        fault: &mut Faults,
    ) -> Result<Outcome> {
        let generation = parent.checked_add(1).ok_or(Error::Conflict)?;
        self.validate_roots()?;
        snapshot.unchanged(source)?;
        validate_payload(snapshot, &self.grant.environment)?;
        let stage = format!("stage-{operation}");
        let final_name = format!("generation-{operation}");
        // All collisions checked before copying; syscall no-replace enforces it
        // again. No retry deletes or guesses the status of incomplete staging.
        for name in [&stage, &final_name, &format!("selector-{operation}")] {
            if self.custody.exists(name)? {
                return Err(Error::Collision);
            }
        }
        if parent == 0 && !fs::names(self.custody.fd())?.is_empty() {
            return Err(Error::Interrupted);
        }
        if self.backups.exists(operation)? || self.backups.exists(&stage)? {
            return Err(Error::Collision);
        }
        snapshot.check_destination(&self.custody)?;
        snapshot.check_destination(&self.backups)?;
        self.custody.sync(fault)?;
        self.backups.sync(fault)?;
        // Independent backup is completed first and never mounted as live data.
        let backup_stage = self.backups.mkdir(&stage)?;
        let backup_data = backup_stage.mkdir("data")?;
        snapshot.copy(source, &backup_data, fault)?;
        let backup = Backup {
            version: FORMAT,
            binding: self.grant.binding.clone(),
            operation: operation.into(),
            environment: self.grant.environment.clone(),
            snapshot_digest: snapshot.digest().into(),
        };
        let backup_bytes = encode(&backup)?;
        backup_stage.write_record("backup.json", &backup_bytes, fault)?;
        backup_stage.sync(fault)?;
        self.backups.sync(fault)?;
        fault.at(Step::BackupRename)?;
        self.backups.rename_new(&stage, &self.backups, operation)?;
        self.backups.sync(fault)?;
        let backup_digest = fs::digest(&backup_bytes);
        let staging = self.custody.mkdir(&stage)?;
        let data = staging.mkdir("data")?;
        snapshot.copy(source, &data, fault)?;
        let r = Record {
            version: FORMAT,
            binding: self.grant.binding.clone(),
            operation: operation.into(),
            generation,
            parent,
            parent_operation: self.selector()?.map(|s| s.operation),
            environment: self.grant.environment.clone(),
            backup: operation.into(),
            backup_digest,
            snapshot_digest: snapshot.digest().into(),
        };
        staging.write_record("receipt.json", &encode(&r)?, fault)?;
        staging.sync(fault)?;
        self.custody.sync(fault)?;
        self.validate_roots()?;
        snapshot.unchanged(source)?;
        fault.at(Step::GenerationRename)?;
        self.custody
            .rename_new(&stage, &self.custody, &final_name)?;
        self.custody.sync(fault)?;
        self.publish(operation, parent, fault)
    }
    fn validate_backup(&self, r: &Record) -> Result<()> {
        let dir = self.backups.child(&r.backup)?;
        dir.revalidate(true)?;
        let bytes = dir.read_record("backup.json")?;
        if fs::digest(&bytes) != r.backup_digest {
            return Err(Error::Corrupt);
        }
        let b: Backup = decode(&bytes)?;
        if b.version != FORMAT
            || b.binding != r.binding
            || b.operation != r.operation
            || b.environment != r.environment
            || b.snapshot_digest != r.snapshot_digest
        {
            return Err(Error::Corrupt);
        }
        if tree::Inventory::scan(&dir.child("data")?)?.digest() != r.snapshot_digest {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
    fn publish(&self, operation: &str, expected: u64, fault: &mut Faults) -> Result<Outcome> {
        if let Some(done) = self.expected(operation, expected)? {
            return Ok(done);
        }
        let (r, data, record_sha256) = self.record(operation)?;
        if r.parent != expected || r.environment != self.grant.environment {
            return Err(Error::Conflict);
        }
        // Unselected candidates are still immutable offline snapshots. Only an
        // already-committed selector may have changed application bytes.
        if tree::Inventory::scan(&data)?.digest() != r.snapshot_digest {
            return Err(Error::Corrupt);
        }
        if self.selector()?.map(|s| s.operation) != r.parent_operation {
            return Err(Error::Conflict);
        }
        self.validate_backup(&r)?;
        let generation = self.custody.child(&format!("generation-{operation}"))?;
        generation.sync_record("receipt.json", fault)?;
        generation.sync(fault)?;
        let s = Selector {
            version: FORMAT,
            binding: r.binding.clone(),
            generation: r.generation,
            operation: operation.into(),
            record_sha256,
        };
        let tmp = format!("selector-{operation}");
        let bytes = encode(&s)?;
        if self.custody.exists(&tmp)? {
            if self.custody.read_record(&tmp)? != bytes {
                return Err(Error::Corrupt);
            }
        } else {
            fault.at(Step::SelectorWrite)?;
            self.custody.write_record(&tmp, &bytes, fault)?;
        }
        self.custody.sync_record(&tmp, fault)?;
        self.custody.sync(fault)?;
        self.validate_roots()?;
        if let Some(done) = self.expected(operation, expected)? {
            return Ok(done);
        }
        fault.at(Step::SelectorRename)?;
        if expected == 0 {
            self.custody
                .rename_new(&tmp, &self.custody, "active.json")?;
        } else {
            self.custody.replace_selector(&tmp)?;
        }
        // After this point failure is AMBIGUOUS, never an automatic rollback.
        // A retry MUST reread selector. Nothing deletes the new/old/backup copy.
        if fault
            .at(Step::SelectorSync)
            .and_then(|_| self.custody.sync(fault))
            .is_err()
        {
            return Err(Error::Uncertain);
        }
        Ok(Outcome::Committed(r.generation))
    }
    /// Recover only complete published generations. Unknown/incomplete staging
    /// remains offline; there is no scan-and-adopt, overwrite or delete fallback.
    pub(super) fn recover(
        &self,
        operation: &str,
        expected: u64,
        fault: &mut Faults,
    ) -> Result<Outcome> {
        if let Some(done) = self.expected(operation, expected)? {
            self.custody.sync(fault)?;
            return Ok(done);
        }
        if self.selector()?.is_none() {
            // Missing active with multiple retained generations is ambiguous.
            // Do not roll the selector back to an older operation automatically.
            let target = format!("generation-{operation}").into_bytes();
            if fs::names(self.custody.fd())?
                .iter()
                .any(|n| n.starts_with(b"generation-") && n != &target)
            {
                return Err(Error::Interrupted);
            }
        }
        if !self.custody.exists(&format!("generation-{operation}"))? {
            return Err(Error::Interrupted);
        }
        self.publish(operation, expected, fault)
    }
    /// Export the requested retained backup into a fresh isolated caller-owned
    /// directory. This does NOT change active selection or erase current writes.
    pub(super) fn rollback_copy(
        &self,
        operation: &str,
        destination: &fs::Dir,
        fault: &mut Faults,
    ) -> Result<()> {
        self.validate_roots()?;
        destination.revalidate(true)?;
        let _lease = destination.lock()?;
        for root in &self.protected {
            if !destination.disjoint(root)? {
                return Err(Error::Custody);
            }
        }
        if !destination.disjoint(&self.custody)?
            || !destination.disjoint(&self.backups)?
            || !destination.disjoint(&self.grant.source)?
        {
            return Err(Error::Custody);
        }
        if !fs::names(destination.fd())?.is_empty() {
            return Err(Error::Collision);
        }
        let (r, _, _) = self.record(operation)?;
        self.validate_backup(&r)?;
        let data = self.backups.child(operation)?.child("data")?;
        let pinned = tree::Inventory::scan(&data)?;
        pinned.copy(&data, destination, fault)?;
        destination.sync(fault)
    }
}
/// Offline selected view; not a boot permit or arbitrary host-path mount. The
/// store borrow/guard must survive any future data writer and all mutators.
pub(super) struct Selected<'s> {
    pub(super) record: Record,
    pub(super) data: fs::Dir,
    _lease: &'s File,
}
fn validate_payload(inventory: &tree::Inventory, environment: &Environment) -> Result<()> {
    let mut expected = match environment.layout {
        Layout::OverlayImage => vec![b"lower.erofs".to_vec(), b"upper.ext4".to_vec()],
        Layout::LegacyClone => vec![b"root".to_vec()],
    };
    if environment.provision == Provision::Bundle {
        expected.push(b"bundle".to_vec());
    }
    expected.sort();
    if inventory.top_names() != expected {
        return Err(Error::Unsupported(
            "incomplete or unknown capture component",
        ));
    }
    if environment.layout == Layout::LegacyClone && !inventory.is_directory(b"root") {
        return Err(Error::Corrupt);
    }
    if environment.provision == Provision::Bundle && !inventory.is_directory(b"bundle") {
        return Err(Error::Corrupt);
    }
    if let Some(lower) = &environment.lower_sha256 {
        if inventory.file_digest(b"lower.erofs") != Some(lower.as_str())
            || inventory.file_digest(b"upper.ext4").is_none()
        {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}
