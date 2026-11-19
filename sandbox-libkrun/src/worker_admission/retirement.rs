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
        Arc,
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
    sentinel: Identity,
    used: AtomicBool,
    never_spawned: bool,
}
impl WriterRetirement {
    pub(super) fn issue(lease: &Lease, runtime: &str) -> io::Result<Self> {
        Ok(Self(Arc::new(RemovalAuthority::issue(
            lease, runtime, false,
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
            sentinel: op.sentinel.clone(),
            used: AtomicBool::new(false),
            never_spawned,
        })
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
        let mut bindings = Vec::new();
        for path in &paths {
            if !path.starts_with(&root) {
                return Err(invalid("cleanup target outside root"));
            }
            let entry = self
                .entries
                .iter()
                .find(|e| {
                    matches!(e.role, PathRole::MutableTree | PathRole::MutableFile)
                        && e.path.get().is_ok_and(|p| p == *path)
                })
                .ok_or_else(|| invalid("cleanup receipt does not cover target"))?;
            super::validate_entry(entry)?;
            if self.never_spawned {
                let tx = lease.0.namespace.transaction()?;
                let op = tx.operation(&lease.0.id)?;
                let target_identity = entry
                    .pins
                    .iter()
                    .find(|(p, _)| p.get().is_ok_and(|p| p == *path))
                    .map(|(_, id)| id);
                let created = op
                    .journals
                    .iter()
                    .filter(|j| j.committed)
                    .flat_map(|j| &j.intents)
                    .any(|intent| {
                        matches!(
                            intent.kind,
                            super::catalog::EffectKind::Create
                                | super::catalog::EffectKind::Publish
                        ) && intent.completed
                            && !intent.cancelled
                            && intent.old.is_none()
                            && intent.to.get().is_ok_and(|p| p == *path)
                            && intent.new.as_ref() == target_identity
                    });
                let already_removed = target_identity.is_none()
                    && op
                        .journals
                        .iter()
                        .filter(|j| j.committed)
                        .flat_map(|j| &j.intents)
                        .any(|i| {
                            i.kind == super::catalog::EffectKind::Remove
                                && i.completed
                                && i.to.get().is_ok_and(|p| p == *path)
                        });
                if !created && !already_removed {
                    return Err(invalid(
                        "never-spawned receipt cannot delete preexisting or legacy data",
                    ));
                }
            }
            bindings.push(entry.clone());
        }
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
            &bindings,
            &self.runtime,
            &self.sentinel,
            self.never_spawned,
        )
    }
}
