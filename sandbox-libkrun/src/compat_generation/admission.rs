// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Inspection never grants a boot. Reuse the offline record contract, don't
//! interpret marker existence or an old project token as migration permission.
use super::{
    Error, FORMAT, Record, Result, Selector, decode, fs,
    store::{Selected, Store},
};
use std::{fs::File, path::Path};

pub(crate) fn inspect(path: &Path, vm_id: &str) -> std::io::Result<()> {
    inspect_private(path, vm_id).map_err(|_| crate::worker_admission::refused())
}
fn inspect_private(path: &Path, vm_id: &str) -> Result<()> {
    let dir = fs::Dir::open(path, true)?;
    let _inspection = dir.lock()?;
    let bytes = dir.read_record("active.json")?;
    let selected: Selector = decode(&bytes)?;
    if selected.version != FORMAT
        || !super::sha(&selected.binding)
        || selected.binding.get(..32) != Some(vm_id)
        || !super::id(&selected.operation)
        || selected.generation == 0
        || !super::sha(&selected.record_sha256)
    {
        return Err(Error::Corrupt);
    }
    let generation = dir.child(&format!("generation-{}", selected.operation))?;
    generation.revalidate(true)?;
    let receipt = generation.read_record("receipt.json")?;
    let record: Record = decode(&receipt)?;
    record.validate(&selected.binding)?;
    if record.operation != selected.operation
        || record.generation != selected.generation
        || fs::digest(&receipt) != selected.record_sha256
    {
        return Err(Error::Corrupt);
    }
    generation.child("data")?.revalidate(true)?;
    // No content scan, syncfs or guest boot here. Full immutable lower/profile
    // selection is performed by Store::selected under its own lifetime lease.
    dir.revalidate(true)
}

/// No public/deserializable constructor. A future local control implementation
/// must retain actual OS-peer ownership, retired writer/credential evidence,
/// sanitization and profile admission. File paths or valid records cannot issue it.
pub(crate) struct TransitionAuthority {
    _offline: super::OfflineGrant,
    _coordinator_custody: File,
}
struct BootAuthority {
    binding: String,
    environment: super::Environment,
    _owner_lease: File,
    _retirement_custody: File,
    _sanitization_custody: File,
    _profile_custody: File,
}
struct Handoff<'a> {
    selected: Selected<'a>,
    _authority: &'a BootAuthority,
    _transition: crate::worker_admission::Lease,
}
/// Prepared internal seam: exact CURRENT selection and both authority/storage
/// lifetimes retained together. No command builder can consume this yet.
fn handoff<'a>(
    store: &'a Store<'a>,
    proof: &'a BootAuthority,
    transition: &TransitionAuthority,
    base: &Path,
    vm_id: &str,
) -> Result<Handoff<'a>> {
    let lease = crate::worker_admission::transition(base, vm_id, transition)?;
    let selected = store.selected()?.ok_or(Error::Interrupted)?;
    if selected.record.binding.get(..32) != Some(vm_id)
        || selected.record.binding != transition._offline.binding
        || selected.record.environment != transition._offline.environment
        || selected.record.binding != proof.binding
        || selected.record.environment != proof.environment
    {
        return Err(Error::Corrupt);
    }
    Ok(Handoff {
        selected,
        _authority: proof,
        _transition: lease,
    })
}

#[cfg(test)]
mod tests;
