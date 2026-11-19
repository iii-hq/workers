// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::{
    Lease, PathRole,
    catalog::{EndpointRecord, EndpointState, HoldKind, overlaps},
    storage::*,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::{SocketAddr, UnixListener},
    },
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SentinelRecord {
    version: u32,
    id: String,
    operation: String,
    epoch: u64,
    namespace: Identity,
}
pub(crate) struct EndpointGuard {
    lease: Lease,
    hold: String,
    path: PathBuf,
    parent: Directory,
    active: bool,
}
pub(crate) struct BoundEndpoint {
    listener: Option<UnixListener>,
    lifetime: Option<EndpointLifetime>,
}
pub(crate) struct EndpointListener {
    listener: Option<tokio::net::UnixListener>,
    lifetime: Option<EndpointLifetime>,
}
struct EndpointLifetime {
    lease: Lease,
    id: String,
    sentinel: Option<File>,
    path: PathBuf,
    parent: Directory,
    identity: Identity,
    closed: bool,
}
pub(super) fn socket_identity(path: &Path, expected_uid: u32) -> io::Result<Option<Identity>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() && m.uid() == expected_uid && m.nlink() == 1 => {
            Ok(Some(Identity::metadata(&m)))
        }
        Ok(_) => Err(invalid("foreign or nonsocket endpoint")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub(super) fn validate_sentinel(
    ns: &super::catalog::Namespace,
    receipt: &EndpointRecord,
) -> io::Result<File> {
    let name = format!("ep-{}", receipt.id);
    let file = match ns.directory.file(&name, false) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => ns.archive.file(&name, false)?,
        Err(e) => return Err(e),
    };
    if Identity::of(&file)? != receipt.sentinel {
        return Err(invalid("endpoint sentinel replaced"));
    }
    receipt.sentinel.regular(ns.uid)?;
    if file.metadata()?.len() > 2048 {
        return Err(limit());
    }
    let record: SentinelRecord = read_json(&file)?;
    if record.version != 1
        || record.id != receipt.id
        || record.operation != receipt.operation
        || record.epoch != receipt.epoch
        || !record
            .namespace
            .same_inode(&Identity::of(&ns.directory.file)?)
    {
        return Err(invalid("endpoint sentinel binding mismatch"));
    }
    Ok(file)
}
// This is the only stale test: independent endpoint ownership, never op holders.
pub(super) fn replaceable(
    tx: &super::catalog::Transaction<'_>,
    ns: &super::catalog::Namespace,
    operation: &super::catalog::Operation,
    path: &Path,
) -> io::Result<bool> {
    let Some(receipt) = tx.catalog.endpoints.iter().find(|e| {
        e.operation == operation.id
            && e.path.get().is_ok_and(|p| p == path)
            && e.state == EndpointState::Bound
    }) else {
        return Ok(false);
    };
    let parent = path.parent().ok_or_else(|| invalid("endpoint parent"))?;
    for e in &operation.entries {
        if e.role != PathRole::Endpoint && overlaps(&e.path.get()?, parent) {
            return Ok(false);
        }
    }
    for h in &operation.holds {
        if h.kind != HoldKind::Endpoint && overlaps(&h.path.get()?, parent) {
            return Ok(false);
        }
    }
    for j in &operation.journals {
        if !j.committed
            && j.footprint
                .iter()
                .any(|p| p.get().is_ok_and(|p| overlaps(&p, parent)))
        {
            return Ok(false);
        }
    }
    let file = validate_sentinel(ns, receipt)?;
    if !ofd(&file, true)? {
        return Ok(false);
    }
    match sentinel(&file) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(e) => Err(e),
    }
}
fn retire_socket(
    tx: &mut super::catalog::Transaction<'_>,
    ns: &super::catalog::Namespace,
    receipt: &EndpointRecord,
    parent: &Directory,
) -> io::Result<()> {
    let path = receipt.path.get()?;
    let identity = receipt
        .identity
        .as_ref()
        .ok_or_else(|| invalid("endpoint bind gap unresolved"))?;
    parent.revalidate()?;
    parent.private(ns.uid)?;
    if !receipt.parent.same_inode(&Identity::of(&parent.file)?)
        || !socket_identity(&path, ns.uid)?.is_some_and(|id| identity.same_inode(&id))
    {
        return Err(invalid("endpoint fingerprint changed before retirement"));
    }
    let slot = tx
        .catalog
        .endpoints
        .iter_mut()
        .find(|e| e.id == receipt.id)
        .ok_or_else(|| invalid("endpoint missing"))?;
    slot.state = EndpointState::CloseIntent;
    tx.save()?;
    parent.revalidate()?;
    if !socket_identity(&path, ns.uid)?.is_some_and(|id| identity.same_inode(&id)) {
        return Err(invalid("endpoint replaced after close intent"));
    }
    unlink_at(
        &parent.file,
        path.file_name().ok_or_else(|| invalid("endpoint name"))?,
    )?;
    parent.file.sync_all()?;
    settle_retired(tx, receipt)?;
    tx.save()
}
pub(super) fn settle_retired(
    tx: &mut super::catalog::Transaction<'_>,
    receipt: &EndpointRecord,
) -> io::Result<()> {
    tx.catalog
        .endpoints
        .iter_mut()
        .find(|e| e.id == receipt.id)
        .ok_or_else(|| invalid("endpoint missing"))?
        .state = EndpointState::Retired;
    let op = tx.operation_mut(&receipt.operation)?;
    op.holds
        .retain(|h| !(h.kind == HoldKind::Endpoint && h.path == receipt.path));
    op.entries
        .retain(|e| !(e.role == PathRole::Endpoint && e.path == receipt.path));
    // Never clear mixed writer uncertainty, mutation holds, journals or pins.
    Ok(())
}
impl EndpointGuard {
    pub(super) fn new(lease: Lease, path: &Path) -> io::Result<Self> {
        let path = absolute(path)?;
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("endpoint parent"))?)?;
        parent.private(uid())?;
        let tx = lease.0.namespace.transaction()?;
        for operation in &tx.catalog.operations {
            for entry in &operation.entries {
                if entry.role != PathRole::Endpoint
                    && overlaps(&entry.path.get()?, &parent.path)
                    && (operation.id == lease.0.id || !operation.closed)
                {
                    return Err(invalid(
                        "endpoint private parent overlaps payload/writer scope",
                    ));
                }
            }
        }
        drop(tx);
        let hold = lease.hold(&path, HoldKind::Endpoint, Some(PathRole::Endpoint))?;
        Ok(Self {
            lease,
            hold,
            path,
            parent,
            active: true,
        })
    }
    pub(crate) fn bind(mut self) -> io::Result<BoundEndpoint> {
        self.parent.revalidate()?;
        let mut tx = self.lease.0.namespace.transaction()?;
        if let Some(old) = socket_identity(&self.path, uid())? {
            let receipt = tx
                .catalog
                .endpoints
                .iter()
                .find(|e| {
                    e.path.get().is_ok_and(|p| p == self.path) && e.state == EndpointState::Bound
                })
                .ok_or_else(|| invalid("unrecorded endpoint"))?
                .clone();
            if !receipt
                .identity
                .as_ref()
                .is_some_and(|id| id.same_inode(&old))
                || !replaceable(
                    &tx,
                    &self.lease.0.namespace,
                    tx.operation(&receipt.operation)?,
                    &self.path,
                )?
            {
                return Err(invalid("live or unresolved endpoint"));
            }
            let prior = validate_sentinel(&self.lease.0.namespace, &receipt)?;
            sentinel(&prior)?;
            retire_socket(&mut tx, &self.lease.0.namespace, &receipt, &self.parent)?;
        }
        if tx.catalog.endpoints.len() >= 256 {
            return Err(limit());
        }
        let op = tx.operation_mut(&self.lease.0.id)?;
        if op.uncertain || op.closed {
            return Err(invalid("operation cannot bind endpoint"));
        }
        op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
        let epoch = op.epoch;
        let id = new_id();
        let sentinel_file = self
            .lease
            .0
            .namespace
            .directory
            .file(&format!("ep-{id}"), true)?;
        sentinel(&sentinel_file)?;
        write_json(
            &sentinel_file,
            &SentinelRecord {
                version: 1,
                id: id.clone(),
                operation: self.lease.0.id.clone(),
                epoch,
                namespace: Identity::of(&self.lease.0.namespace.directory.file)?,
            },
        )?;
        tx.catalog.endpoints.push(EndpointRecord {
            id: id.clone(),
            path: HostPath::new(&self.path)?,
            identity: None,
            operation: self.lease.0.id.clone(),
            epoch,
            sentinel: Identity::of(&sentinel_file)?,
            parent: Identity::of(&self.parent.file)?,
            state: EndpointState::BindIntent,
        });
        tx.save()?;
        self.lease.0.namespace.directory.file.sync_all()?;
        drop(tx);
        self.parent.revalidate()?;
        let listener = UnixListener::bind(&self.path)?;
        self.parent.revalidate()?;
        let identity =
            socket_identity(&self.path, uid())?.ok_or_else(|| invalid("bound socket missing"))?;
        self.parent.file.sync_all()?;
        let mut tx = self.lease.0.namespace.transaction()?;
        let receipt = tx
            .catalog
            .endpoints
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| invalid("bind intent missing"))?;
        receipt.identity = Some(identity.clone());
        receipt.state = EndpointState::Bound;
        for e in &mut tx.operation_mut(&self.lease.0.id)?.entries {
            if e.role == PathRole::Endpoint && e.path.get()? == self.path {
                e.pins = super::pins(&self.path, PathRole::Endpoint)?;
            }
        }
        tx.save()?;
        drop(tx);
        self.active = false;
        Ok(BoundEndpoint {
            listener: Some(listener),
            lifetime: Some(EndpointLifetime {
                lease: self.lease.clone(),
                id,
                sentinel: Some(sentinel_file),
                path: self.path.clone(),
                parent: Directory::open(&self.parent.path)?,
                identity,
                closed: false,
            }),
        })
    }
}
impl Drop for EndpointGuard {
    fn drop(&mut self) {
        if self.active {
            // Preserve a durable bind/close gap, without poisoning unrelated mixed data.
            if let Ok(mut tx) = self.lease.0.namespace.transaction() {
                let unresolved = tx.catalog.endpoints.iter().any(|e| {
                    e.operation == self.lease.0.id
                        && e.path.get().is_ok_and(|p| p == self.path)
                        && e.state != EndpointState::Retired
                });
                if !unresolved {
                    if let Ok(op) = tx.operation_mut(&self.lease.0.id) {
                        op.holds.retain(|h| h.id != self.hold);
                        let _ = tx.save();
                    }
                }
            }
        }
    }
}
impl BoundEndpoint {
    pub(crate) fn into_async(mut self) -> io::Result<EndpointListener> {
        let listener = self
            .listener
            .take()
            .ok_or_else(|| invalid("listener consumed"))?;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::UnixListener::from_std(listener)?;
        Ok(EndpointListener {
            listener: Some(listener),
            lifetime: self.lifetime.take(),
        })
    }
    pub(crate) fn close(mut self) -> io::Result<()> {
        drop(self.listener.take());
        self.lifetime
            .take()
            .ok_or_else(|| invalid("endpoint lifetime missing"))?
            .close()
    }
}
impl EndpointListener {
    pub(crate) async fn accept(&self) -> io::Result<(tokio::net::UnixStream, SocketAddr)> {
        let (stream, address) = self
            .listener
            .as_ref()
            .ok_or_else(|| invalid("listener consumed"))?
            .accept()
            .await?;
        Ok((stream, address.into()))
    }
    pub(crate) fn close(mut self) -> io::Result<()> {
        drop(self.listener.take());
        self.lifetime
            .take()
            .ok_or_else(|| invalid("endpoint lifetime missing"))?
            .close()
    }
}
impl EndpointLifetime {
    fn close(mut self) -> io::Result<()> {
        let mut tx = self.lease.0.namespace.transaction()?;
        let receipt = tx
            .catalog
            .endpoints
            .iter()
            .find(|e| e.id == self.id)
            .ok_or_else(|| invalid("endpoint receipt missing"))?
            .clone();
        if receipt.state != EndpointState::Bound
            || receipt.identity.as_ref() != Some(&self.identity)
            || receipt.path.get()? != self.path
            || self
                .sentinel
                .as_ref()
                .map(Identity::of)
                .transpose()?
                .as_ref()
                != Some(&receipt.sentinel)
        {
            return Err(invalid("endpoint lifetime mismatch"));
        }
        retire_socket(&mut tx, &self.lease.0.namespace, &receipt, &self.parent)?;
        self.closed = true;
        Ok(())
    }
}
impl Drop for EndpointLifetime {
    fn drop(&mut self) {
        if !self.closed {
            if let Some(file) = self.sentinel.take() {
                std::mem::forget(file);
            }
        }
    }
}
// Fields drop in listener-before-lifetime order. No unlink on unknown cancellation.
#[cfg(test)]
pub(super) fn abandon_closed_test_listener(mut bound: BoundEndpoint) {
    drop(bound.listener.take());
    if let Some(mut lifetime) = bound.lifetime.take() {
        lifetime.closed = true;
        drop(lifetime.sentinel.take());
    }
}
