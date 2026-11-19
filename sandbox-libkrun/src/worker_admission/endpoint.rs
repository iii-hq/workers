// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::{
    Lease, PathRole,
    catalog::{EndpointRecord, HoldKind, overlaps},
    storage::*,
};
use std::{
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};
pub(crate) struct EndpointGuard {
    lease: Option<Lease>,
    hold: String,
    path: PathBuf,
    parent: Directory,
}
pub(crate) struct BoundEndpoint {
    listener: UnixListener,
    lifetime: EndpointLifetime,
}
pub(crate) struct EndpointLifetime {
    lease: Option<Lease>,
    hold: String,
    path: PathBuf,
    parent: Directory,
    identity: Identity,
    closed: bool,
}
fn socket_identity(path: &Path) -> io::Result<Option<Identity>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() || metadata.uid() != uid() || metadata.nlink() != 1
            {
                return Err(invalid("foreign or nonsocket endpoint"));
            }
            Ok(Some(Identity::metadata(&metadata)))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
impl EndpointGuard {
    pub(super) fn new(lease: Lease, path: &Path) -> io::Result<Self> {
        let path = absolute(path)?;
        let parent = Directory::open(
            path.parent()
                .ok_or_else(|| invalid("endpoint parent missing"))?,
        )?;
        parent.private(uid())?;
        let tx = lease.0.namespace.transaction()?;
        let op = tx.operation(&lease.0.id)?;
        // Endpoint parent must be isolated from payload/export scopes. Endpoint
        // role alone is not permission to place a socket in a guest share.
        for e in &op.entries {
            if e.role != PathRole::Endpoint && overlaps(&e.path.get()?, &parent.path) {
                return Err(invalid("endpoint parent overlaps admitted payload"));
            }
        }
        drop(tx);
        let hold = lease.hold(&path, HoldKind::Endpoint, Some(PathRole::Endpoint))?;
        Ok(Self {
            lease: Some(lease),
            hold,
            path,
            parent,
        })
    }
    pub(crate) fn bind(mut self) -> io::Result<BoundEndpoint> {
        let lease = self
            .lease
            .as_ref()
            .ok_or_else(|| invalid("endpoint already consumed"))?;
        self.parent.revalidate()?;
        self.parent.private(uid())?;
        let old = socket_identity(&self.path)?;
        let mut tx = lease.0.namespace.transaction()?;
        if let Some(old) = old {
            let receipt = tx
                .catalog
                .endpoints
                .iter()
                .find(|e| e.path.get().is_ok_and(|p| p == self.path))
                .ok_or_else(|| invalid("unrecorded endpoint cannot be removed"))?
                .clone();
            if !receipt.identity.same_inode(&old) || receipt.operation == lease.0.id {
                return Err(invalid("endpoint receipt mismatch"));
            }
            let prior = tx.operation(&receipt.operation)?;
            if prior.uncertain
                || prior.entries.iter().any(|e| e.role != PathRole::Endpoint)
                || !prior.journals.is_empty()
            {
                return Err(invalid(
                    "previous endpoint operation has unresolved writer history",
                ));
            }
            let sentinel_file = lease
                .0
                .namespace
                .directory
                .file(&format!("op-{}", receipt.operation), false)?;
            // Fingerprint was checked first. Nonblocking ownership of the old
            // sentinel is the only stale proof; never probe/connect to a socket.
            sentinel(&sentinel_file)?;
            self.parent.revalidate()?;
            if !socket_identity(&self.path)?.is_some_and(|id| old.same_inode(&id)) {
                return Err(invalid("endpoint changed before stale cleanup"));
            }
            unlink_at(
                &self.parent.file,
                self.path
                    .file_name()
                    .ok_or_else(|| invalid("endpoint name"))?,
            )?;
            self.parent.file.sync_all()?;
            tx.operation_mut(&receipt.operation)?.closed = true;
            tx.catalog.endpoints.retain(|e| e.path != receipt.path);
        }
        let op = tx.operation_mut(&lease.0.id)?;
        // Persist uncertainty BEFORE bind; receipt publication clears it. Any
        // failure in the gap leaves the operation fenced, never auto-unlinked.
        op.uncertain = true;
        op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
        let epoch = op.epoch;
        tx.save()?;
        drop(tx);
        self.parent.revalidate()?;
        let listener = UnixListener::bind(&self.path)?;
        self.parent.revalidate()?;
        let identity =
            socket_identity(&self.path)?.ok_or_else(|| invalid("bound endpoint missing"))?;
        self.parent.file.sync_all()?;
        let mut tx = lease.0.namespace.transaction()?;
        if tx.catalog.endpoints.len() >= 256 {
            return Err(limit());
        }
        tx.catalog.endpoints.push(EndpointRecord {
            path: HostPath::new(&self.path)?,
            identity: identity.clone(),
            operation: lease.0.id.clone(),
            epoch,
        });
        let op = tx.operation_mut(&lease.0.id)?;
        op.uncertain = false;
        for e in &mut op.entries {
            if e.path.get()? == self.path {
                e.pins = super::pins(&self.path, PathRole::Endpoint)?;
            }
        }
        tx.save()?;
        drop(tx);
        let lease = self
            .lease
            .take()
            .ok_or_else(|| invalid("endpoint consumed"))?;
        let lifetime = EndpointLifetime {
            lease: Some(lease),
            hold: self.hold.clone(),
            path: self.path.clone(),
            parent: Directory::open(&self.parent.path)?,
            identity,
            closed: false,
        };
        Ok(BoundEndpoint { listener, lifetime })
    }
}
impl Drop for EndpointGuard {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let _ = lease.finish_hold(&self.hold, true);
        }
    }
}
impl BoundEndpoint {
    pub(crate) fn into_parts(self) -> (UnixListener, EndpointLifetime) {
        (self.listener, self.lifetime)
    }
}
impl EndpointLifetime {
    /// Caller must have closed ALL listener clones/tasks before this call.
    /// Keeping the returned lifetime alongside every listener owner is mandatory.
    pub(crate) fn close(mut self) -> io::Result<()> {
        let lease = self
            .lease
            .as_ref()
            .ok_or_else(|| invalid("endpoint lifetime consumed"))?;
        self.parent.revalidate()?;
        self.parent.private(uid())?;
        let mut tx = lease.0.namespace.transaction()?;
        let receipt = tx
            .catalog
            .endpoints
            .iter()
            .find(|e| e.operation == lease.0.id && e.path.get().is_ok_and(|p| p == self.path))
            .ok_or_else(|| invalid("endpoint receipt missing"))?;
        if receipt.identity != self.identity
            || !socket_identity(&self.path)?.is_some_and(|id| self.identity.same_inode(&id))
        {
            return Err(invalid("endpoint replaced before close"));
        }
        unlink_at(
            &self.parent.file,
            self.path
                .file_name()
                .ok_or_else(|| invalid("endpoint name"))?,
        )?;
        self.parent.file.sync_all()?;
        tx.catalog
            .endpoints
            .retain(|e| !(e.operation == lease.0.id && e.path.get().is_ok_and(|p| p == self.path)));
        let op = tx.operation_mut(&lease.0.id)?;
        op.holds.retain(|h| h.id != self.hold);
        for entry in &mut op.entries {
            if entry.path.get()? == self.path {
                entry
                    .pins
                    .retain(|(p, _)| p.get().is_ok_and(|p| p != self.path));
            }
        }
        tx.save()?;
        self.closed = true;
        Ok(())
    }
}
#[cfg(test)]
pub(super) fn abandon_closed_test_listener(mut lifetime: EndpointLifetime) {
    // Test harness has already dropped its only listener. Do not expose this
    // crash simulation in production or use it as a migration authority.
    lifetime.closed = true;
    drop(lifetime.lease.take());
}
impl Drop for EndpointLifetime {
    fn drop(&mut self) {
        if !self.closed {
            if let Some(lease) = self.lease.take() {
                // Unknown listener ownership retains the sentinel. No Drop unlink.
                std::mem::forget(lease);
            }
        }
    }
}
