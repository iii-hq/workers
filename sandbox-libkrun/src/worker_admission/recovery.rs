// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Explicit recovery is not retirement authority. No fallback to catalog.previous.
use super::{
    catalog::{
        ArchiveRecord, Catalog, EffectKind, EndpointRecord, EndpointState, Namespace, Operation,
        RecoveryIntent, overlaps,
    },
    storage::*,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io, os::fd::AsRawFd, path::Path, sync::Arc};
const MAX_ARCHIVES: usize = 512;
#[derive(Clone, Copy, Debug)]
pub(crate) enum RecoveryAction {
    ReconcileVerified,
    ArchiveCompleted,
}
pub(crate) struct RecoveryRequest {
    pub operation_id: String,
    pub expected_record_sha256: String,
    pub action: RecoveryAction,
}
#[derive(Debug, Serialize)]
pub(crate) struct RecoveryReport {
    version: u32,
    uid: u32,
    status: String,
    records: Vec<RecoveryFact>,
    archives: usize,
}
#[derive(Debug, Serialize)]
struct RecoveryFact {
    operation_id: String,
    record_sha256: String,
    state: String,
    journals: usize,
    endpoints: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveBundle {
    version: u32,
    operation: Operation,
    endpoints: Vec<EndpointRecord>,
    runtimes: Vec<super::catalog::RuntimeRecord>,
    record_sha256: String,
}
pub(super) fn digest<T: Serialize>(value: &T) -> io::Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value).map_err(io::Error::other)?,
    )))
}
pub(crate) fn inspect_recovery_for_uid(uid: u32) -> io::Result<RecoveryReport> {
    inspect(&Namespace::administrative(uid)?)
}
pub(crate) fn recover_for_uid(uid: u32, request: RecoveryRequest) -> io::Result<RecoveryReport> {
    recover(&Namespace::administrative(uid)?, request)
}
#[cfg(test)]
pub(super) fn inspect_test(context: &super::AdmissionContext) -> io::Result<RecoveryReport> {
    inspect(&context.namespace)
}
#[cfg(test)]
pub(super) fn recover_test(
    context: &super::AdmissionContext,
    request: RecoveryRequest,
) -> io::Result<RecoveryReport> {
    recover(&context.namespace, request)
}
fn report(ns: &Namespace, catalog: &Catalog, status: &str) -> io::Result<RecoveryReport> {
    let records = catalog
        .operations
        .iter()
        .map(|op| {
            Ok(RecoveryFact {
                operation_id: op.id.clone(),
                record_sha256: digest(op)?,
                state: if op.closed {
                    "closed"
                } else if op.uncertain {
                    "uncertain"
                } else {
                    "open"
                }
                .into(),
                journals: op.journals.len(),
                endpoints: catalog
                    .endpoints
                    .iter()
                    .filter(|e| e.operation == op.id)
                    .count(),
            })
        })
        .collect::<io::Result<_>>()?;
    Ok(RecoveryReport {
        version: 1,
        uid: ns.uid,
        status: status.into(),
        records,
        archives: catalog.archives.len(),
    })
}
fn inspect(ns: &Arc<Namespace>) -> io::Result<RecoveryReport> {
    let tx = ns.transaction()?;
    report(ns, &tx.catalog, "inspected")
}
fn identity(path: &Path) -> io::Result<Option<Identity>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            if m.file_type().is_symlink() {
                return Err(invalid("recovery refuses symlink"));
            }
            Ok(Some(Identity::metadata(&m)))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn matches(actual: &Option<Identity>, expected: &Option<Identity>) -> bool {
    match (actual, expected) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_inode(b) && a.links == b.links,
        _ => false,
    }
}
fn parent(path: &Path, expected: &Identity) -> io::Result<Directory> {
    let directory = Directory::open(path.parent().ok_or_else(|| invalid("recovery parent"))?)?;
    if !expected.same_inode(&Identity::of(&directory.file)?) {
        return Err(invalid("recovery parent replaced"));
    }
    Ok(directory)
}
fn operation_free(tx: &super::catalog::Transaction<'_>, op: &Operation) -> io::Result<Vec<File>> {
    let ns = tx.namespace();
    let sentinel_file = tx.operation_sentinel(op)?;
    if Identity::of(&sentinel_file)? != op.sentinel {
        return Err(invalid("recovery sentinel mismatch"));
    }
    sentinel(&sentinel_file)?;
    let mut files = vec![sentinel_file];
    for endpoint in tx.catalog.endpoints.iter().filter(|e| e.operation == op.id) {
        let file = super::endpoint::validate_sentinel(ns, endpoint)?;
        sentinel(&file)?;
        files.push(file);
    }
    for runtime in tx.catalog.runtimes.iter().filter(|r| r.operation == op.id) {
        let file = super::runtime::validate_sentinel(ns, runtime)?;
        sentinel(&file)?;
        files.push(file);
    }
    // Explicitly archived operations already have no active footprints. Otherwise
    // refuse even an independently live reader of a recovery footprint.
    for other in &tx.catalog.operations {
        if other.id == op.id || other.closed {
            continue;
        }
        if other.entries.iter().any(|a| {
            op.entries
                .iter()
                .any(|b| match (a.path.get(), b.path.get()) {
                    (Ok(a), Ok(b)) => overlaps(&a, &b),
                    _ => true,
                })
        }) {
            return Err(busy());
        }
    }
    Ok(files)
}
#[cfg(test)]
pub(super) fn recover_fault_test(
    context: &super::AdmissionContext,
    request: RecoveryRequest,
    at: &str,
) -> io::Result<RecoveryReport> {
    recover_inner(&context.namespace, request, Some(at))
}
fn recover(ns: &Arc<Namespace>, request: RecoveryRequest) -> io::Result<RecoveryReport> {
    recover_inner(ns, request, None)
}
fn boundary(fault: Option<&str>, at: &str) -> io::Result<()> {
    if fault == Some(at) {
        return Err(io::Error::other("injected archive boundary fault"));
    }
    Ok(())
}
fn recover_inner(
    ns: &Arc<Namespace>,
    request: RecoveryRequest,
    fault: Option<&str>,
) -> io::Result<RecoveryReport> {
    if !id_valid(&request.operation_id) || request.expected_record_sha256.len() != 64 {
        return Err(invalid("invalid recovery request"));
    }
    let mut tx = ns.transaction()?;
    let original = tx.operation(&request.operation_id)?.clone();
    if digest(&original)? != request.expected_record_sha256 {
        return Err(invalid("recovery digest compare-and-swap mismatch"));
    }
    let _sentinels = operation_free(&tx, &original)?;
    match request.action {
        RecoveryAction::ArchiveCompleted => archive(ns, &mut tx, &original, fault)?,
        RecoveryAction::ReconcileVerified => {
            if original
                .cache
                .as_ref()
                .is_some_and(|c| c.phase == super::catalog::CachePhase::Preparing)
            {
                return report(
                    ns,
                    &tx.catalog,
                    "unresolved: abandoned cache preparation requires explicit producer recovery; no read transfer is inferred",
                );
            }
            if original.holders > 1
                || original
                    .holds
                    .iter()
                    .any(|h| h.kind == super::catalog::HoldKind::Read)
            {
                return report(
                    ns,
                    &tx.catalog,
                    "unresolved: unverified holder/read lifetime",
                );
            }
            if tx.catalog.runtimes.iter().any(|r| {
                r.operation == original.id
                    && !matches!(
                        r.state,
                        super::catalog::RuntimeState::Retired
                            | super::catalog::RuntimeState::Cancelled
                    )
            }) {
                return report(
                    ns,
                    &tx.catalog,
                    "unresolved: runtime writer requires real owned wait",
                );
            }
            let mut resolved = original.clone();
            if !original.closed && original.journals.is_empty() {
                return report(
                    ns,
                    &tx.catalog,
                    "unresolved: no durable writer completion evidence",
                );
            }
            // Endpoint effects are independently proven. CloseIntent plus
            // matching retained parent and absence can finish a recorded unlink;
            // unknown bind gaps and still-present endpoints remain unresolved.
            let endpoint_receipts: Vec<_> = tx
                .catalog
                .endpoints
                .iter()
                .filter(|e| e.operation == original.id && e.state != EndpointState::Retired)
                .cloned()
                .collect();
            for endpoint in &endpoint_receipts {
                if endpoint.state != EndpointState::CloseIntent
                    || endpoint.identity.is_none()
                    || parent(&endpoint.path.get()?, &endpoint.parent).is_err()
                    || identity(&endpoint.path.get()?)?.is_some()
                {
                    return report(ns, &tx.catalog, "unresolved: endpoint retirement required");
                }
            }
            tx.operation_mut(&original.id)?.recovery = Some(RecoveryIntent {
                record_sha256: request.expected_record_sha256.clone(),
                completed: false,
            });
            tx.save()?;
            for endpoint in &endpoint_receipts {
                let path = endpoint.path.get()?;
                let directory = parent(&path, &endpoint.parent)?;
                if identity(&path)?.is_some() {
                    return report(
                        ns,
                        &tx.catalog,
                        "unresolved: endpoint appeared during close recovery",
                    );
                }
                directory.file.sync_all()?;
                super::endpoint::settle_retired(&mut tx, endpoint)?;
                resolved.holds.retain(|h| {
                    !(h.kind == super::catalog::HoldKind::Endpoint
                        && endpoint.hold_id.as_ref() == Some(&h.id)
                        && h.owner == endpoint.owner)
                });
                resolved
                    .entries
                    .retain(|e| !(e.role == super::PathRole::Endpoint && e.path == endpoint.path));
            }
            tx.save()?;
            for journal in &mut resolved.journals {
                if journal.committed {
                    continue;
                } // Historical lineage, not old pathname equality.
                if journal.plan == "owner-read"
                    && !super::owner_read::restore_verified(journal, ns.uid)?
                {
                    return report(
                        ns,
                        &tx.catalog,
                        "unresolved: permission restoration not proven",
                    );
                }
                if !verify_journal(journal)? {
                    return report(
                        ns,
                        &tx.catalog,
                        "unresolved: incomplete or ambiguous journal postconditions",
                    );
                }
            }
            if resolved.holds.iter().any(|h| {
                !matches!(
                    h.kind,
                    super::catalog::HoldKind::Mutation | super::catalog::HoldKind::OwnerRead
                ) || !resolved
                    .journals
                    .iter()
                    .any(|j| j.id == h.id && j.committed)
            }) {
                return report(ns, &tx.catalog, "unresolved: hold lacks a resolved journal");
            }
            // Completion follows the durable recovery intent and all proofs.
            resolved.recovery = Some(RecoveryIntent {
                record_sha256: request.expected_record_sha256,
                completed: true,
            });
            resolved.holders = 0;
            resolved.holder_ids.clear();
            resolved.holds.clear();
            resolved.uncertain = false;
            resolved.closed = true;
            *tx.operation_mut(&original.id)? = resolved;
            tx.save()?;
        }
    }
    report(ns, &tx.catalog, "completed")
}
fn verify_journal(j: &mut super::catalog::Journal) -> io::Result<bool> {
    if j.permissions
        .iter()
        .any(|p| !p.restored || !p.acl_captured || !p.acl_before_grant)
    {
        return Ok(false);
    }
    if j.intents.is_empty()
        && (!j.artifacts.is_empty() || !j.changes.is_empty() || !j.disposals.is_empty())
    {
        return Ok(false);
    }
    // Validate every last placement. Earlier stage placement may be consumed
    // only by a later exact publication or matching disposal in this journal.
    for index in 0..j.intents.len() {
        let intent = j.intents[index].clone();
        if intent.cancelled {
            continue;
        }
        let to = intent.to.get()?;
        let target_parent = match parent(&to, &intent.to_parent) {
            Ok(p) => p,
            Err(_) => return Ok(false),
        };
        if let Some(from) = &intent.from {
            let from = from.get()?;
            if let Some(expected) = &intent.from_parent {
                if parent(&from, expected).is_err() {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }
        let consumed = j.intents[index + 1..]
            .iter()
            .any(|later| later.from.as_ref() == Some(&intent.to) && later.new == intent.new)
            || j.disposals
                .iter()
                .any(|d| d.path == intent.to && Some(&d.file) == intent.new.as_ref());
        if intent.kind == EffectKind::Stage && consumed {
            continue;
        }
        match intent.kind {
            EffectKind::Create | EffectKind::Stage | EffectKind::Diagnostic => {
                let actual = identity(&to)?;
                if !intent.completed || intent.new.is_none() || !matches(&actual, &intent.new) {
                    return Ok(false);
                }
                if intent.kind == EffectKind::Stage {
                    return Ok(false);
                } // unpublished stage is not silently disposed
            }
            EffectKind::Publish => {
                let from = intent
                    .from
                    .as_ref()
                    .ok_or_else(|| invalid("publication lacks stage"))?
                    .get()?;
                let actual = identity(&to)?;
                let stage = identity(&from)?;
                let prior = intent
                    .prior
                    .as_ref()
                    .map(|p| identity(&p.get()?))
                    .transpose()?
                    .flatten();
                if matches(&actual, &intent.new) && stage.is_none() && matches(&prior, &intent.old)
                {
                    // Newly published bytes are the only accepted completed path.
                    if let Some(id) = &actual {
                        if id.mode & libc::S_IFMT == libc::S_IFREG {
                            open_at(
                                &target_parent.file,
                                to.file_name().ok_or_else(|| invalid("target name"))?,
                                libc::O_RDONLY,
                                0,
                            )?
                            .sync_all()?;
                        }
                    }
                } else if matches(&actual, &intent.old)
                    && matches(&stage, &intent.new)
                    && prior.is_none()
                {
                    // The entirely retained old placement is safe but the owned
                    // new stage remains evidence. Mark cancellation, never delete it.
                    j.intents[index].cancelled = true;
                    j.intents[index].completed = false;
                } else {
                    return Ok(false);
                }
            }
            EffectKind::Remove => {
                if identity(&to)?.is_some() {
                    return Ok(false);
                }
            }
            EffectKind::Adopt => {
                let from = intent
                    .from
                    .as_ref()
                    .ok_or_else(|| invalid("adopt lacks source"))?
                    .get()?;
                let source = identity(&from)?;
                let target = identity(&to)?;
                if source.is_none() && matches(&target, &intent.new) {
                } else if matches(&source, &intent.new) && target.is_none() {
                    j.intents[index].cancelled = true;
                    j.intents[index].completed = false;
                } else {
                    return Ok(false);
                }
            }
        }
        target_parent.file.sync_all()?;
        if !j.intents[index].cancelled {
            j.intents[index].completed = true;
        }
    }
    for disposal in &mut j.disposals {
        let path = disposal.path.get()?;
        let directory = match parent(&path, &disposal.parent) {
            Ok(p) => p,
            Err(_) => return Ok(false),
        };
        match identity(&path)? {
            None => {
                directory.file.sync_all()?;
                disposal.discarded = true;
            }
            Some(id) if id.same_inode(&disposal.file) && id.links == 1 => return Ok(false), // no deletion retry without new explicit operation
            Some(_) => return Ok(false),
        }
    }
    // Every generated artifact must be represented by a before-effect intent,
    // a preserved-prior placement or a matching diagnostic record.
    if j.artifacts.iter().any(|p| {
        !j.intents
            .iter()
            .any(|i| &i.to == p || i.prior.as_ref() == Some(p))
    }) {
        return Ok(false);
    }
    j.committed = true;
    Ok(true)
}
pub(super) fn validate_archives(ns: &Namespace, catalog: &Catalog) -> io::Result<()> {
    if catalog.archives.len() > MAX_ARCHIVES {
        return Err(limit());
    }
    let mut allowed = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for archive in &catalog.archives {
        if !id_valid(&archive.operation)
            || !ids.insert(&archive.operation)
            || archive.digest.len() != 64
            || archive.endpoints.len() > 256
            || archive.runtimes.len() > 256
        {
            return Err(invalid("invalid archive index"));
        }
        for name in [
            format!("{}.stage", archive.operation),
            format!("{}.json", archive.operation),
            format!("op-{}", archive.operation),
        ] {
            if ns.archive.path.join(&name).try_exists()? {
                allowed.push(name);
            }
        }
        for endpoint in &archive.endpoints {
            let n = format!("ep-{}", endpoint.id);
            if ns.archive.path.join(&n).try_exists()? {
                allowed.push(n);
            }
        }
        for runtime in &archive.runtimes {
            let name = format!("rt-{}", runtime.id);
            if ns.archive.path.join(&name).try_exists()? {
                allowed.push(name);
            }
        }
        let final_path = ns.archive.path.join(format!("{}.json", archive.operation));
        if final_path.try_exists()? {
            let file = ns
                .archive
                .file(&format!("{}.json", archive.operation), false)?;
            Identity::of(&file)?.regular(ns.uid)?;
            if archive
                .metadata
                .as_ref()
                .is_some_and(|id| Identity::of(&file).is_ok_and(|actual| &actual != id))
            {
                return Err(invalid("archive metadata replaced"));
            }
            let bundle: ArchiveBundle = read_json(&file)?;
            if bundle.version != 1
                || bundle.operation.id != archive.operation
                || bundle.record_sha256 != archive.digest
                || digest(&bundle.operation)? != archive.digest
                || bundle.operation.sentinel != archive.sentinel
                || digest(&bundle.endpoints)? != digest(&archive.endpoints)?
                || digest(&bundle.runtimes)? != digest(&archive.runtimes)?
            {
                return Err(invalid("archive bundle mismatch"));
            }
        } else if archive.complete {
            return Err(invalid("completed archive metadata missing"));
        }
        if archive.complete
            && catalog
                .operations
                .iter()
                .any(|op| op.id == archive.operation)
        {
            return Err(invalid("archived/live operation duplication"));
        }
        if !archive.complete
            && !catalog
                .operations
                .iter()
                .any(|op| op.id == archive.operation)
        {
            return Err(invalid("incomplete archive lost its live operation"));
        }
        if archive.complete {
            for runtime in &archive.runtimes {
                if Identity::of(&ns.archive.file(&format!("rt-{}", runtime.id), false)?)?
                    != runtime.sentinel
                {
                    return Err(invalid("archived runtime sentinel replaced"));
                }
            }
            let file = ns
                .archive
                .file(&format!("op-{}", archive.operation), false)?;
            if Identity::of(&file)? != archive.sentinel {
                return Err(invalid("archived sentinel replaced"));
            }
            for endpoint in &archive.endpoints {
                let file = ns.archive.file(&format!("ep-{}", endpoint.id), false)?;
                if Identity::of(&file)? != endpoint.sentinel {
                    return Err(invalid("archived endpoint sentinel replaced"));
                }
            }
        }
    }
    let names = ns.archive.names()?;
    let mut bytes = 0u64;
    for name in &names {
        let file = ns.archive.file(name, false)?;
        Identity::of(&file)?.regular(ns.uid)?;
        bytes = bytes
            .checked_add(file.metadata()?.len())
            .ok_or_else(limit)?;
        if bytes > 128 * 1024 * 1024 {
            return Err(limit());
        }
    }
    if names.len() != allowed.len() || names.iter().any(|n| !allowed.contains(n)) {
        return Err(invalid("unknown archive material"));
    }
    Ok(())
}
fn rename_noreplace(
    from: &Directory,
    source: &str,
    to: &Directory,
    target: &str,
) -> io::Result<()> {
    from.revalidate()?;
    to.revalidate()?;
    let source = super::storage::name(source.as_ref())?;
    let target = super::storage::name(target.as_ref())?;
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
            "archive requires NOREPLACE",
        ))
    }
}
fn archive(
    ns: &Namespace,
    tx: &mut super::catalog::Transaction<'_>,
    op: &Operation,
    fault: Option<&str>,
) -> io::Result<()> {
    if !op.closed
        || op.uncertain
        || op.holders != 0
        || !op.holds.is_empty()
        || op
            .journals
            .iter()
            .any(|j| !j.committed || j.disposals.iter().any(|d| !d.discarded))
        || op.recovery.as_ref().is_some_and(|r| !r.completed)
    {
        return Err(invalid("only resolved terminal operations can be archived"));
    }
    let endpoints: Vec<_> = tx
        .catalog
        .endpoints
        .iter()
        .filter(|e| e.operation == op.id)
        .cloned()
        .collect();
    if endpoints.iter().any(|e| e.state != EndpointState::Retired) {
        return Err(busy());
    }
    let runtimes: Vec<_> = tx
        .catalog
        .runtimes
        .iter()
        .filter(|r| r.operation == op.id)
        .cloned()
        .collect();
    if runtimes.iter().any(|r| {
        !matches!(
            r.state,
            super::catalog::RuntimeState::Retired | super::catalog::RuntimeState::Cancelled
        )
    }) {
        return Err(busy());
    }
    let sha = digest(op)?;
    if let Some(prior) = tx.catalog.archives.iter().find(|a| a.operation == op.id) {
        if prior.digest != sha {
            return Err(invalid("archive intent digest mismatch"));
        }
    } else {
        if tx.catalog.archives.len() >= MAX_ARCHIVES {
            return Err(limit());
        }
        tx.catalog.archives.push(ArchiveRecord {
            operation: op.id.clone(),
            digest: sha.clone(),
            metadata: None,
            sentinel: op.sentinel.clone(),
            endpoints: endpoints.clone(),
            runtimes: runtimes.clone(),
            complete: false,
        });
        tx.save()?;
    }
    boundary(fault, "archive-intent")?;
    let existing_bytes =
        ns.archive
            .names()?
            .iter()
            .try_fold(0u64, |total, name| -> io::Result<u64> {
                Ok(total
                    .checked_add(ns.archive.file(name, false)?.metadata()?.len())
                    .ok_or_else(limit)?)
            })?;
    // Envelope escaping may double the serialized payload; reserve the full
    // per-record cap before staging, rather than crossing the archive limit.
    if existing_bytes
        .checked_add(MAX_BYTES as u64)
        .ok_or_else(limit)?
        > 128 * 1024 * 1024
    {
        return Err(limit());
    }
    let stage_name = format!("{}.stage", op.id);
    let final_name = format!("{}.json", op.id);
    if !ns.archive.path.join(&final_name).try_exists()? {
        let file = match ns.archive.file(&stage_name, true) {
            Ok(file) => {
                if uid() == 0 {
                    syscall(unsafe { libc::fchown(file.as_raw_fd(), ns.uid, !0) })?;
                }
                write_json(
                    &file,
                    &ArchiveBundle {
                        version: 1,
                        operation: op.clone(),
                        endpoints: endpoints.clone(),
                        runtimes: runtimes.clone(),
                        record_sha256: sha.clone(),
                    },
                )?;
                file
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let file = ns.archive.file(&stage_name, false)?;
                let bundle: ArchiveBundle = read_json(&file)?;
                if bundle.record_sha256 != sha || digest(&bundle.operation)? != sha {
                    return Err(invalid("archive stage cannot be resumed"));
                }
                file
            }
            Err(e) => return Err(e),
        };
        Identity::of(&file)?.regular(ns.uid)?;
        file.sync_all()?;
        ns.archive.file.sync_all()?;
        boundary(fault, "archive-stage")?;
        rename_noreplace(&ns.archive, &stage_name, &ns.archive, &final_name)?;
        ns.archive.file.sync_all()?;
    }
    boundary(fault, "archive-promoted")?;
    let metadata_file = ns.archive.file(&final_name, false)?;
    let bundle: ArchiveBundle = read_json(&metadata_file)?;
    if bundle.version != 1
        || bundle.record_sha256 != sha
        || digest(&bundle.operation)? != sha
        || digest(&bundle.endpoints)? != digest(&endpoints)?
        || digest(&bundle.runtimes)? != digest(&runtimes)?
    {
        return Err(invalid("archive metadata does not match frozen operation"));
    }
    let metadata = Identity::of(&metadata_file)?;
    metadata.regular(ns.uid)?;
    tx.catalog
        .archives
        .iter_mut()
        .find(|a| a.operation == op.id)
        .ok_or_else(|| invalid("archive intent missing"))?
        .metadata = Some(metadata);
    tx.save()?;
    boundary(fault, "archive-receipt")?;
    // Archive receipt is durable before any sentinel or live-row removal.
    let mut names = vec![format!("op-{}", op.id)];
    names.extend(endpoints.iter().map(|e| format!("ep-{}", e.id)));
    names.extend(runtimes.iter().map(|r| format!("rt-{}", r.id)));
    for n in names {
        if ns.directory.path.join(&n).try_exists()? {
            let expected = if n == format!("op-{}", op.id) {
                &op.sentinel
            } else if n.starts_with("rt-") {
                &runtimes
                    .iter()
                    .find(|r| n == format!("rt-{}", r.id))
                    .ok_or_else(|| invalid("unbound runtime sentinel move"))?
                    .sentinel
            } else {
                &endpoints
                    .iter()
                    .find(|e| n == format!("ep-{}", e.id))
                    .ok_or_else(|| invalid("unbound sentinel move"))?
                    .sentinel
            };
            if Identity::of(&ns.directory.file(&n, false)?)? != *expected {
                return Err(invalid("sentinel replaced before archive move"));
            }
            rename_noreplace(&ns.directory, &n, &ns.archive, &n)?;
            ns.archive.file.sync_all()?;
            ns.directory.file.sync_all()?;
        }
    }
    boundary(fault, "archive-sentinels")?;
    tx.catalog.operations.retain(|p| p.id != op.id);
    tx.catalog.endpoints.retain(|e| e.operation != op.id);
    tx.catalog.runtimes.retain(|r| r.operation != op.id);
    tx.catalog
        .archives
        .iter_mut()
        .find(|a| a.operation == op.id)
        .ok_or_else(|| invalid("archive intent missing"))?
        .complete = true;
    tx.save()
}
