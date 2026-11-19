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
    entries: Vec<Entry>,
    lineage: super::lineage::Anchor,
    sentinel: Identity,
    used: AtomicBool,
    never_spawned: bool,
    endpoints: Vec<super::catalog::EndpointRecord>,
    endpoint_gate: Mutex<()>,
}
impl WriterRetirement {
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
