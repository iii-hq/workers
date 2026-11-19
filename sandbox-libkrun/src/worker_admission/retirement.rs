// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! A status is not cleanup authority. These receipts have no public constructor
//! or Deserialize, retain their operation, and are consumed once by exact Remove.
use super::{
    Lease, MutationGuard, MutationPlan, PathRole,
    catalog::{Entry, RuntimeState},
    storage::*,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone)]
pub(crate) struct WriterRetirement(Arc<RemovalAuthority>);
pub(crate) struct NeverSpawned(Arc<RemovalAuthority>);
struct RemovalAuthority {
    lease: Lease,
    runtime: String,
    epoch: u64,
    runtime_history: String,
    entries: Vec<Entry>,
    lineage: super::lineage::Anchor,
    sentinel: Identity,
    used: AtomicBool,
    never_spawned: bool,
    endpoints: Vec<super::catalog::EndpointRecord>,
    endpoint_gate: Mutex<()>,
}
#[derive(Clone)]
pub(super) struct CacheContinuation {
    receipt: WriterRetirement,
    consumer: Lease,
    binding: super::catalog::CacheContinuationBinding,
}
fn runtime_history(tx: &super::catalog::Transaction<'_>, operation: &str) -> io::Result<String> {
    super::recovery::digest(
        &tx.catalog
            .runtimes
            .iter()
            .filter(|r| r.operation == operation)
            .collect::<Vec<_>>(),
    )
}
fn consumer_history(tx: &super::catalog::Transaction<'_>, lease: &Lease) -> io::Result<String> {
    let op = tx.operation(&lease.0.id)?;
    let endpoints = tx
        .catalog
        .endpoints
        .iter()
        .filter(|e| e.operation == op.id)
        .collect::<Vec<_>>();
    super::recovery::digest(&(
        op.epoch,
        &op.entries,
        &op.journals,
        &op.cache_reads,
        &op.logs,
        &op.recovery,
        &op.holder_ids,
        &op.holds,
        runtime_history(tx, &op.id)?,
        endpoints,
    ))
}
impl CacheContinuation {
    pub(super) fn binding(&self) -> super::catalog::CacheContinuationBinding {
        self.binding.clone()
    }
    pub(super) fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, ()>> {
        self.receipt
            .0
            .endpoint_gate
            .lock()
            .map_err(|_| invalid("retirement continuation mutex poisoned"))
    }
    pub(super) fn validate(
        &self,
        consumer: &Lease,
        tx: &super::catalog::Transaction<'_>,
    ) -> io::Result<()> {
        if !Arc::ptr_eq(&self.consumer.0, &consumer.0) {
            return Err(invalid(
                "cache continuation requires original lifecycle Arc",
            ));
        }
        self.receipt.0.validate_cache_continuation(consumer, tx)?;
        if runtime_history(tx, &consumer.0.id)? != self.binding.runtime_history_sha256
            || consumer_history(tx, consumer)? != self.binding.consumer_state_sha256
        {
            return Err(invalid("cache continuation snapshot became stale"));
        }
        Ok(())
    }
    pub(super) fn validate_record(
        &self,
        cache: &super::catalog::CacheRecord,
        tx: &super::catalog::Transaction<'_>,
    ) -> io::Result<()> {
        let metadata = cache
            .continuation
            .as_ref()
            .ok_or_else(|| invalid("continuation metadata missing"))?;
        if cache.consumer != self.consumer.0.id
            || metadata.runtime != self.receipt.0.runtime
            || metadata.runtime_epoch != self.receipt.0.epoch
            || super::recovery::digest(metadata)? != super::recovery::digest(&self.binding)?
        {
            return Err(invalid("cache continuation private binding mismatch"));
        }
        self.validate(&self.consumer, tx)
    }
}
impl WriterRetirement {
    pub(crate) fn probe_cache(
        &self,
        lease: &Lease,
        entries: &[super::CacheEntry<'_>],
    ) -> io::Result<super::CacheProbe> {
        let capability = self.cache_continuation(lease)?;
        super::shared_cache::probe_continuation(lease, entries, capability)
    }
    pub(crate) fn prepare_cache(
        &self,
        lease: &Lease,
        entries: &[super::CacheEntry<'_>],
    ) -> io::Result<super::CachePreparation> {
        let capability = self.cache_continuation(lease)?;
        super::shared_cache::prepare_continuation(lease, entries, capability)
    }
    fn cache_continuation(&self, lease: &Lease) -> io::Result<CacheContinuation> {
        let _gate = self
            .0
            .endpoint_gate
            .lock()
            .map_err(|_| invalid("retirement continuation mutex poisoned"))?;
        let tx = lease.0.namespace.transaction()?;
        self.0.validate_cache_continuation(lease, &tx)?;
        Ok(CacheContinuation {
            receipt: self.clone(),
            consumer: lease.clone(),
            binding: super::catalog::CacheContinuationBinding {
                runtime: self.0.runtime.clone(),
                runtime_epoch: self.0.epoch,
                runtime_history_sha256: runtime_history(&tx, &lease.0.id)?,
                consumer_state_sha256: consumer_history(&tx, lease)?,
            },
        })
    }
    pub(super) fn issue(lease: &Lease, runtime: &str) -> io::Result<Self> {
        Ok(Self(Arc::new(RemovalAuthority::issue(
            lease, runtime, false,
        )?)))
    }
    pub(crate) fn retire_runtime_endpoints(
        &self,
        lease: &Lease,
        paths: &[&Path],
    ) -> io::Result<()> {
        self.0.retire_endpoints(
            lease,
            paths,
            #[cfg(test)]
            None,
        )
    }
    #[cfg(test)]
    pub(super) fn retire_endpoints_fault(
        &self,
        lease: &Lease,
        paths: &[&Path],
        fault: &str,
    ) -> io::Result<()> {
        self.0.retire_endpoints(lease, paths, Some(fault))
    }
    pub(crate) fn begin_remove(
        &self,
        lease: &Lease,
        root: &Path,
        targets: &[&Path],
    ) -> io::Result<MutationGuard> {
        self.0.begin_remove(lease, root, targets)
    }
}
impl NeverSpawned {
    pub(super) fn issue(lease: &Lease, runtime: &str) -> io::Result<Self> {
        Ok(Self(Arc::new(RemovalAuthority::issue(
            lease, runtime, true,
        )?)))
    }
    pub(crate) fn begin_remove(
        &self,
        lease: &Lease,
        root: &Path,
        targets: &[&Path],
    ) -> io::Result<MutationGuard> {
        self.0.begin_remove(lease, root, targets)
    }
}
impl RemovalAuthority {
    fn validate_cache_continuation(
        &self,
        lease: &Lease,
        tx: &super::catalog::Transaction<'_>,
    ) -> io::Result<()> {
        if self.never_spawned
            || self.used.load(Ordering::Acquire)
            || !Arc::ptr_eq(&self.lease.0, &lease.0)
            || lease.0.holder != lease.0.id
        {
            return Err(invalid(
                "cache continuation needs current unused private retirement",
            ));
        }
        let op = tx.operation(&lease.0.id)?;
        if op.closed
            || op.uncertain
            || op.transition
            || op.cache.is_some()
            || op.sentinel != self.sentinel
            || Identity::of(&lease.0.sentinel)? != self.sentinel
            || op.holder_ids != vec![lease.0.holder.clone()]
            || op.holders != 1
            || !op.holds.is_empty()
            || op.recovery.as_ref().is_some_and(|r| !r.completed)
        {
            return Err(invalid(
                "retired lifecycle has unresolved ownership or recovery",
            ));
        }
        let runtimes = tx
            .catalog
            .runtimes
            .iter()
            .filter(|r| r.operation == op.id)
            .collect::<Vec<_>>();
        if runtimes.iter().map(|r| r.epoch).max() != Some(self.epoch)
            || runtime_history(tx, &op.id)? != self.runtime_history
        {
            return Err(invalid(
                "a later or altered runtime invalidates cache continuation",
            ));
        }
        let current = runtimes
            .iter()
            .find(|r| r.id == self.runtime)
            .ok_or_else(|| invalid("retirement runtime missing"))?;
        if current.state != RuntimeState::Retired
            || !current.writer_retired
            || current.epoch != self.epoch
        {
            return Err(invalid(
                "receipt does not represent latest actual retirement",
            ));
        }
        for r in runtimes {
            let clean = r.activity.phase == super::catalog::ActivityPhase::Clean
                && r.activity.failure.is_none()
                && !r.guest_consistent;
            let verified = match r.state {
                RuntimeState::Retired => r.writer_retired && r.admission_holder.is_some(),
                RuntimeState::Cancelled => {
                    !r.writer_retired && r.admission_holder.is_none() && r.activity.sequence == 0
                }
                _ => false,
            };
            if !clean || !verified {
                return Err(invalid(
                    "unverified terminal runtime in continuation history",
                ));
            }
        }
        if tx
            .catalog
            .endpoints
            .iter()
            .filter(|e| e.operation == op.id)
            .any(|e| {
                e.state != super::catalog::EndpointState::Retired
                    || e.owner.is_none()
                    || e.hold_id.is_none()
                    || e.identity.is_none()
            })
        {
            return Err(invalid(
                "all exact endpoints must retire before cache continuation",
            ));
        }
        if op
            .logs
            .iter()
            .any(|r| r.phase != super::log::LogPhase::Finished || r.pending.is_some())
        {
            return Err(invalid("log writers must finish before cache continuation"));
        }
        if op.journals.iter().any(|j| {
            !j.committed
                || j.intents.iter().any(|i| !i.completed || i.cancelled)
                || j.disposals.iter().any(|d| !d.discarded)
                || j.permissions
                    .iter()
                    .any(|p| !p.restored || !p.acl_captured || !p.acl_before_grant)
        }) {
            return Err(invalid(
                "unfinished mutation or permission restoration blocks cache continuation",
            ));
        }
        let targets = self
            .entries
            .iter()
            .filter(|e| matches!(e.role, PathRole::MutableTree | PathRole::MutableFile))
            .map(|e| e.path.get())
            .collect::<io::Result<Vec<_>>>()?;
        super::lineage::validate(&self.lineage, &self.entries, op, &targets, false)?;
        // Strict facts in the original receipt cannot be rewritten by a later
        // cache append. Current added facts remain separately protected/proven.
        for old in self.entries.iter().filter(|e| e.role == PathRole::Strict) {
            if !op
                .entries
                .iter()
                .any(|e| e.role == PathRole::Strict && e.path == old.path && e.pins == old.pins)
            {
                return Err(invalid("original Strict claim changed after retirement"));
            }
        }
        for entry in &op.entries {
            if !self
                .entries
                .iter()
                .any(|original| original.path == entry.path && original.role == entry.role)
            {
                let proved = entry.role == PathRole::Strict
                    && op.cache_reads.iter().any(|receipt| {
                        receipt.entries.iter().any(|read| {
                            read.path == entry.path
                                && read.role == PathRole::Strict
                                && read.pins == entry.pins
                        })
                    });
                let artifact = entry.role == PathRole::MutableTree
                    && op.journals.iter().any(|j| {
                        j.committed
                            && j.artifacts.contains(&entry.path)
                            && j.intents.iter().any(|i| {
                                i.completed
                                    && !i.cancelled
                                    && (i.to == entry.path || i.prior.as_ref() == Some(&entry.path))
                            })
                            && entry.path.get().is_ok_and(|p| {
                                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                                    n.starts_with(&format!(".iii-stage-{}-", j.id))
                                        || n.starts_with(&format!(".iii-prior-{}-", j.id))
                                })
                            })
                    });
                if !proved && !artifact {
                    return Err(invalid(
                        "cache continuation cannot bless a new mutable or unproved lifecycle declaration",
                    ));
                }
            }
            super::validate_entry(entry)?;
        }
        Ok(())
    }
    fn issue(lease: &Lease, runtime: &str, never_spawned: bool) -> io::Result<Self> {
        let tx = lease.0.namespace.transaction()?;
        let op = tx.operation(&lease.0.id)?;
        let record = tx
            .catalog
            .runtimes
            .iter()
            .find(|r| r.id == runtime && r.operation == lease.0.id)
            .ok_or_else(|| invalid("cleanup receipt runtime missing"))?;
        let required = if never_spawned {
            RuntimeState::Cancelled
        } else {
            RuntimeState::Retired
        };
        if record.state != required
            || (!never_spawned && !record.writer_retired)
            || op.closed
            || op.uncertain
        {
            return Err(invalid(
                "operation is not fully eligible for cleanup receipt",
            ));
        }
        if tx
            .catalog
            .runtimes
            .iter()
            .filter(|r| r.operation == lease.0.id)
            .any(|r| {
                if never_spawned {
                    r.state != RuntimeState::Cancelled
                } else {
                    !matches!(r.state, RuntimeState::Retired | RuntimeState::Cancelled)
                }
            })
        {
            return Err(invalid("another writer is unresolved"));
        }
        for entry in &op.entries {
            if !op.holds.iter().any(|h| {
                h.path.get().is_ok_and(|p| {
                    entry
                        .path
                        .get()
                        .is_ok_and(|q| super::catalog::overlaps(&p, &q))
                })
            }) {
                super::validate_entry(entry)?;
            }
        }
        Ok(Self {
            lease: lease.clone(),
            runtime: runtime.into(),
            runtime_history: runtime_history(&tx, &lease.0.id)?,
            epoch: tx
                .catalog
                .runtimes
                .iter()
                .filter(|r| r.operation == lease.0.id)
                .map(|r| r.epoch)
                .max()
                .ok_or_else(|| invalid("runtime epoch missing"))?,
            entries: op.entries.clone(),
            lineage: super::lineage::Anchor::capture(op, &op.entries)?,
            sentinel: op.sentinel.clone(),
            used: AtomicBool::new(false),
            never_spawned,
            endpoints: tx
                .catalog
                .endpoints
                .iter()
                .filter(|e| {
                    e.operation == lease.0.id
                        && e.owner
                            .as_ref()
                            .and_then(|o| o.runtime.as_ref())
                            .is_some_and(|r| r.id == record.id && r.epoch == record.epoch)
                })
                .cloned()
                .collect(),
            endpoint_gate: Mutex::new(()),
        })
    }
    fn retire_endpoints(
        &self,
        lease: &Lease,
        paths: &[&Path],
        #[cfg(test)] fault: Option<&str>,
    ) -> io::Result<()> {
        let _gate = self
            .endpoint_gate
            .lock()
            .map_err(|_| invalid("endpoint settlement mutex poisoned"))?;
        if self.never_spawned
            || self.used.load(Ordering::Acquire)
            || !Arc::ptr_eq(&self.lease.0, &lease.0)
        {
            return Err(invalid("foreign, used or nonretirement endpoint receipt"));
        }
        if paths.len() > 256 || paths.iter().any(|p| !p.is_absolute()) {
            return Err(invalid("endpoint set must be bounded and absolute"));
        }
        let paths = paths
            .iter()
            .map(|p| absolute(p))
            .collect::<io::Result<Vec<_>>>()?;
        if paths.iter().collect::<std::collections::HashSet<_>>().len() != paths.len() {
            return Err(invalid("duplicate endpoint path"));
        }
        let mut tx = lease.0.namespace.transaction()?;
        let op = tx.operation(&lease.0.id)?;
        if op.closed
            || op.uncertain
            || op.sentinel != self.sentinel
            || tx
                .catalog
                .runtimes
                .iter()
                .filter(|r| r.operation == lease.0.id)
                .map(|r| r.epoch)
                .max()
                != Some(self.epoch)
        {
            return Err(invalid("stale endpoint retirement operation"));
        }
        let runtime = tx
            .catalog
            .runtimes
            .iter()
            .find(|r| r.operation == lease.0.id && r.id == self.runtime)
            .ok_or_else(|| invalid("runtime receipt missing"))?;
        if runtime.state != RuntimeState::Retired
            || !runtime.writer_retired
            || runtime.guest_consistent
            || runtime.activity.phase != super::catalog::ActivityPhase::Clean
            || runtime.activity.failure.is_some()
        {
            return Err(invalid("runtime retirement is not clean/current"));
        }
        let mut selected = Vec::new();
        for path in &paths {
            let anchored = self
                .endpoints
                .iter()
                .find(|e| e.path.get().is_ok_and(|p| &p == path))
                .ok_or_else(|| invalid("endpoint not bound to original retirement receipt"))?;
            let current = tx
                .catalog
                .endpoints
                .iter()
                .find(|e| e.id == anchored.id)
                .ok_or_else(|| invalid("anchored endpoint record disappeared"))?;
            if current.operation != anchored.operation
                || current.path != anchored.path
                || current.epoch != anchored.epoch
                || current.owner != anchored.owner
                || current.hold_id != anchored.hold_id
                || current.sentinel != anchored.sentinel
                || current.parent != anchored.parent
                || current.identity != anchored.identity
                || current.owner.as_ref().is_none_or(|o| {
                    o.holder != runtime.admission_holder.as_deref().unwrap_or("")
                        || o.runtime
                            .as_ref()
                            .is_none_or(|r| r.id != runtime.id || r.epoch != runtime.epoch)
                })
            {
                return Err(invalid("endpoint receipt binding changed"));
            }
            if !self
                .entries
                .iter()
                .any(|e| e.path == current.path && e.role == PathRole::Endpoint)
                && anchored.state != super::catalog::EndpointState::Retired
            {
                return Err(invalid("endpoint was not declared in original receipt"));
            }
            if tx.catalog.endpoints.iter().any(|e| {
                e.operation == lease.0.id
                    && e.path == anchored.path
                    && e.id != anchored.id
                    && e.epoch > anchored.epoch
            }) {
                return Err(invalid("endpoint path has a newer issuance"));
            }
            let parent = Directory::open(path.parent().ok_or_else(|| invalid("endpoint parent"))?)?;
            super::endpoint::prevalidate_retirement(current, &parent, lease.0.namespace.uid)?;
            let sentinel_file = super::endpoint::validate_sentinel(&lease.0.namespace, current)?;
            super::storage::sentinel(&sentinel_file)?;
            // Any independent surviving writer/use overlapping private control
            // storage blocks settlement, even though this runtime exited.
            for operation in &tx.catalog.operations {
                if operation.closed && operation.id != lease.0.id {
                    continue;
                }
                if operation.entries.iter().any(|e| {
                    e.role != PathRole::Endpoint
                        && e.path
                            .get()
                            .is_ok_and(|p| super::catalog::overlaps(&p, &parent.path))
                }) || operation.holds.iter().any(|h| {
                    h.kind != super::catalog::HoldKind::Endpoint
                        && h.path
                            .get()
                            .is_ok_and(|p| super::catalog::overlaps(&p, &parent.path))
                }) {
                    return Err(invalid("endpoint parent overlaps unresolved host resource"));
                }
            }
            selected.push((current.clone(), parent, sentinel_file));
        }
        // Prevalidate ALL requests before any unlink. Keep every independent
        // sentinel lock until the entire bounded operation returns.
        for (index, (record, parent, _sentinel)) in selected.iter().enumerate() {
            super::endpoint::finish_recorded_close(
                &mut tx,
                &lease.0.namespace,
                record,
                parent,
                #[cfg(test)]
                fault,
            )?;
            #[cfg(test)]
            if fault == Some("after-first") && index == 0 {
                return Err(io::Error::other("injected partial endpoint set completion"));
            }
            let _ = index;
        }
        Ok(())
    }
    fn begin_remove(
        &self,
        lease: &Lease,
        root: &Path,
        targets: &[&Path],
    ) -> io::Result<MutationGuard> {
        if !Arc::ptr_eq(&self.lease.0, &lease.0) || self.lease.0.id != lease.0.id {
            return Err(invalid("foreign operation/context cleanup receipt"));
        }
        if targets.is_empty() || targets.len() > 256 {
            return Err(limit());
        }
        let root = absolute(root)?;
        let paths = targets
            .iter()
            .map(|p| absolute(p))
            .collect::<io::Result<Vec<PathBuf>>>()?;
        // Only scope/Arc checks occur before the one-shot claim. Frozen pins
        // are not read against the current path here: owned effects may already
        // have replaced them. Replay, path validation and hold acquisition happen
        // atomically under the catalog transaction in begin_authorized.
        for path in &paths {
            if !path.starts_with(&root)
                || !self.entries.iter().any(|e| {
                    matches!(e.role, PathRole::MutableTree | PathRole::MutableFile)
                        && e.path.get().is_ok_and(|p| p == *path)
                })
            {
                return Err(invalid(
                    "cleanup receipt does not cover original exact target",
                ));
            }
        }
        let _gate = self
            .endpoint_gate
            .lock()
            .map_err(|_| invalid("endpoint settlement mutex poisoned"))?;
        if self
            .used
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(invalid("cleanup receipt already consumed"));
        }
        // Failure consumes the in-memory receipt too. No retry can legitimize a
        // stale layout. Atomic epoch/digest check occurs inside guard acquisition.
        super::mutation::MutationGuard::begin_authorized(
            lease.clone(),
            MutationPlan::Remove {
                root,
                targets: paths,
            },
            self.epoch,
            &self.entries,
            &self.lineage,
            &self.runtime,
            &self.sentinel,
            self.never_spawned,
        )
    }
}
