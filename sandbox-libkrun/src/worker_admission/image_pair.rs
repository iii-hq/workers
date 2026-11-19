// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Exact post-retirement image publication, not a plan-name whitelist.
//!
//! A committed `image-pair` journal advances a private retirement lineage only
//! when it is the concrete producer protocol: three core-issued stages whose
//! writer completion was proven by an independent nonblocking OFD lock (no
//! surviving writer clone), the `image.spool` stage discarded first, then
//! `image.erofs` and `image.id` published in that order from exactly those
//! stages, with `image.id` naming the completed image digest. Every target and
//! the root must already be in the ORIGINAL receipt declaration. Current-path
//! observation never upgrades a receipt: replay in `lineage` still validates the
//! last derived identities, so an external replacement is refused.
use super::{
    PathRole,
    catalog::{EffectKind, Entry, ImagePairProof, Journal},
    storage::*,
};
use sha2::{Digest, Sha256};
use std::{io, os::unix::fs::FileExt};
fn regular(id: &Identity) -> bool {
    id.mode & libc::S_IFMT == libc::S_IFREG
        && id.links == 1
        && id.uid == uid()
        && id.mode & 0o022 == 0
}
/// Structural proof from issued stages, effects and recorded completions only.
/// `Ok(None)`: not the concrete producer protocol (no post-retirement authority).
/// `Err`: the protocol was started but its chain is incomplete or inconsistent.
fn proof(j: &Journal) -> io::Result<Option<ImagePairProof>> {
    if j.plan != "image-pair" || j.payload.is_some() || !j.permissions.is_empty() {
        return Ok(None);
    }
    let root = j.root.get()?;
    let image = HostPath::new(&root.join("image.erofs"))?;
    let id = HostPath::new(&root.join("image.id"))?;
    let spool = HostPath::new(&root.join("image.spool"))?;
    let published = j
        .intents
        .iter()
        .enumerate()
        .filter(|(_, i)| i.kind == EffectKind::Publish)
        .collect::<Vec<_>>();
    if published.len() != 2 || published[0].1.to != image || published[1].1.to != id {
        return Ok(None);
    }
    // Generic ImagePair callers without a spool disposal may still commit, but
    // receive NO post-retirement pair authority. Only the concrete producer
    // with all three completed file descriptions enters this proof protocol.
    if j.disposals.is_empty() {
        return Ok(None);
    }
    if j.footprint != vec![j.root.clone()]
        || j.disposals.len() != 1
        || j.stage_completions.len() != 3
        || j.intents.iter().any(|i| {
            !i.completed
                || i.cancelled
                || !matches!(
                    i.kind,
                    EffectKind::Stage | EffectKind::Publish | EffectKind::Create
                )
        })
    {
        return Err(invalid("incomplete or extraneous ImagePair effects"));
    }
    let creates = j
        .intents
        .iter()
        .filter(|i| i.kind == EffectKind::Create)
        .collect::<Vec<_>>();
    if creates.len() > 1
        || creates.iter().any(|i| {
            i.to != j.root
                || i.new
                    .as_ref()
                    .is_none_or(|id| id.mode & libc::S_IFMT != libc::S_IFDIR)
        })
    {
        return Err(invalid(
            "ImagePair may create only its originally declared root directory",
        ));
    }
    if j.intents
        .iter()
        .filter(|i| i.kind == EffectKind::Stage)
        .count()
        != 3
    {
        return Err(invalid("ImagePair unaccounted stage"));
    }
    let disposal = &j.disposals[0];
    if !disposal.discarded || disposal.slot != spool {
        return Err(invalid("ImagePair spool not exactly discarded"));
    }
    // Consumption order encodes completion before publication: joined spool,
    // joined image writer, joined id writer. No raw File clone may remain.
    let completions = &j.stage_completions;
    if completions[0].slot != spool
        || completions[1].slot != image
        || completions[2].slot != id
        || completions[0].stage != disposal.path
        || completions[0].identity != disposal.file
    {
        return Err(invalid("ImagePair completion order/binding mismatch"));
    }
    let mut identities = std::collections::BTreeSet::new();
    for (index, done) in completions.iter().enumerate() {
        if !regular(&done.identity)
            || !identities.insert((done.identity.dev, done.identity.ino))
            || !j.artifacts.contains(&done.stage)
        {
            return Err(invalid("invalid ImagePair stage identity"));
        }
        let issued = j
            .intents
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind == EffectKind::Stage && i.to == done.stage)
            .collect::<Vec<_>>();
        if issued.len() != 1
            || issued[0].1.new.as_ref() != Some(&done.identity)
            || issued[0].1.old.is_some()
        {
            return Err(invalid("stage completion lacks its issued creation"));
        }
        if index > 0 {
            let (at, publish) = published[index - 1];
            if issued[0].0 >= at
                || publish.from.as_ref() != Some(&done.stage)
                || publish.new.as_ref() != Some(&done.identity)
                || publish.old.as_ref().is_some_and(|i| !regular(i))
            {
                return Err(invalid("ImagePair Stage/Publish chain mismatch"));
            }
        }
    }
    let expected = format!("sha256:{}\n", completions[1].sha256);
    if completions[2].bytes != expected.len() as u64
        || completions[2].sha256 != hex::encode(Sha256::digest(expected.as_bytes()))
    {
        return Err(invalid("ImagePair id is not the completed image digest"));
    }
    Ok(Some(ImagePairProof {
        image,
        id,
        spool,
        image_stage: completions[1].stage.clone(),
        id_stage: completions[2].stage.clone(),
        spool_stage: completions[0].stage.clone(),
        image_sha256: completions[1].sha256.clone(),
    }))
}
/// Called by `MutationGuard::commit` OUTSIDE the catalog transaction while the
/// original mutation still holds the whole footprint: recheck the exact
/// published inodes and bytes, then flush them. No hash-based issuer and no
/// acceptance of a later replacement: the inode must be the completed stage.
pub(super) fn completed_pair(j: &Journal) -> io::Result<Option<ImagePairProof>> {
    let Some(proof) = proof(j)? else {
        return Ok(None);
    };
    for path in [&proof.image, &proof.id] {
        let path = path.get()?;
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("image parent"))?)?;
        let file = open_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("image name"))?,
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        )?;
        let done = j
            .stage_completions
            .iter()
            .find(|c| c.slot.get().is_ok_and(|p| p == path))
            .ok_or_else(|| invalid("ImagePair completion missing"))?;
        if Identity::of(&file)? != done.identity || file.metadata()?.len() != done.bytes {
            return Err(invalid("ImagePair changed before commit"));
        }
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 65536];
        let mut at = 0u64;
        while at < done.bytes {
            let n = (done.bytes - at).min(bytes.len() as u64) as usize;
            file.read_exact_at(&mut bytes[..n], at)?;
            hash.update(&bytes[..n]);
            at += n as u64;
        }
        if hex::encode(hash.finalize()) != done.sha256 {
            return Err(invalid("ImagePair bytes changed before commit"));
        }
        file.sync_all()?;
    }
    Ok(Some(proof))
}
/// Lineage gate for a post-retirement `image-pair` journal. The generic replay
/// in `lineage::validate` still applies every intent and validates the final
/// derived identities; this only admits the journal kind when it is exact.
pub(super) fn validate_lineage(j: &Journal, original: &[Entry]) -> io::Result<()> {
    let derived = proof(j)?.ok_or_else(|| invalid("journal is not a completed exact ImagePair"))?;
    let recorded = j
        .image_pair
        .as_ref()
        .ok_or_else(|| invalid("ImagePair writer completion receipt absent"))?;
    if super::recovery::digest(recorded)? != super::recovery::digest(&derived)? {
        return Err(invalid("ImagePair completion proof changed"));
    }
    for path in [&derived.image, &derived.id, &derived.spool] {
        if !original
            .iter()
            .any(|e| &e.path == path && e.role == PathRole::MutableFile)
        {
            return Err(invalid(
                "ImagePair target was not in original exact receipt",
            ));
        }
    }
    if !original
        .iter()
        .any(|e| e.path == j.root && e.role == PathRole::MutableTree)
    {
        return Err(invalid("ImagePair root was not originally declared"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "image_pair_tests.rs"]
mod tests;
