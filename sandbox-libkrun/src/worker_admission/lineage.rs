// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Receipt-local replay of completed core effects, never a rebase to filesystem
//! observations. The caller holds the same catalog transaction as Remove admission.
use super::{
    PathRole,
    catalog::{EffectIntent, EffectKind, Entry, Journal, Operation, overlaps},
    storage::*,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};
const MAX_NODES: usize = 16384;

pub(super) struct Anchor {
    epoch: u64,
    committed: Vec<(String, u64, String)>,
    pending: Vec<(String, u64)>,
    created: BTreeSet<PathBuf>,
}
impl Anchor {
    pub(super) fn capture(op: &Operation, entries: &[Entry]) -> io::Result<Self> {
        let committed = op
            .journals
            .iter()
            .filter(|j| j.committed)
            .map(|j| Ok((j.id.clone(), j.epoch, super::recovery::digest(j)?)))
            .collect::<io::Result<Vec<_>>>()?;
        let mut created = BTreeSet::new();
        for entry in entries {
            let path = entry.path.get()?;
            let identity = entry
                .pins
                .iter()
                .find(|(p, _)| p == &entry.path)
                .map(|(_, id)| id);
            let own = op
                .journals
                .iter()
                .filter(|j| j.committed)
                .flat_map(|j| &j.intents)
                .any(|i| {
                    i.completed
                        && !i.cancelled
                        && i.old.is_none()
                        && i.to == entry.path
                        && matches!(i.kind, EffectKind::Create | EffectKind::Publish)
                        && i.new.as_ref() == identity
                        && identity.is_some()
                });
            let removed = identity.is_none()
                && op
                    .journals
                    .iter()
                    .filter(|j| j.committed)
                    .flat_map(|j| &j.intents)
                    .any(|i| {
                        i.kind == EffectKind::Remove
                            && i.completed
                            && !i.cancelled
                            && i.to == entry.path
                    });
            if own || removed {
                created.insert(path);
            }
        }
        let pending = op
            .journals
            .iter()
            .filter(|j| !j.committed)
            .map(|j| (j.id.clone(), j.epoch))
            .collect();
        Ok(Self {
            epoch: op.epoch,
            committed,
            pending,
            created,
        })
    }
}
#[derive(Clone)]
struct Node {
    identity: Option<Identity>,
    owned: bool,
}
struct Replay {
    nodes: BTreeMap<PathBuf, Node>,
    touched: BTreeSet<PathBuf>,
}
fn same(a: &Identity, b: &Identity) -> bool {
    a.same_inode(b) && (a.mode & libc::S_IFMT == libc::S_IFDIR || a.links == b.links)
}
fn equal(a: Option<&Identity>, b: Option<&Identity>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same(a, b),
        (None, None) => true,
        _ => false,
    }
}
impl Replay {
    fn put(&mut self, path: PathBuf, node: Node) -> io::Result<()> {
        if !self.nodes.contains_key(&path) && self.nodes.len() >= MAX_NODES {
            return Err(limit());
        }
        self.nodes.insert(path, node);
        Ok(())
    }
    fn initial(entries: &[Entry], anchor: &Anchor) -> io::Result<Self> {
        let mut replay = Self {
            nodes: BTreeMap::new(),
            touched: BTreeSet::new(),
        };
        for entry in entries {
            for (path, identity) in &entry.pins {
                let path = path.get()?;
                if let Some(old) = replay.nodes.get(&path) {
                    if !equal(old.identity.as_ref(), Some(identity)) {
                        return Err(invalid("inconsistent receipt identity snapshots"));
                    }
                } else {
                    replay.put(
                        path.clone(),
                        Node {
                            identity: Some(identity.clone()),
                            owned: anchor.created.contains(&path),
                        },
                    )?;
                }
            }
        }
        // A missing tail was part of the original admission, not new authority.
        for entry in entries {
            let path = entry.path.get()?;
            for ancestor in path.ancestors() {
                if replay.nodes.contains_key(ancestor) {
                    continue;
                }
                replay.put(
                    ancestor.to_path_buf(),
                    Node {
                        identity: None,
                        owned: anchor.created.contains(ancestor),
                    },
                )?;
            }
        }
        Ok(replay)
    }
    fn parent(&self, path: &Path, expected: &Identity) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid("lineage effect parent missing"))?;
        let identity = self
            .nodes
            .get(parent)
            .and_then(|n| n.identity.as_ref())
            .ok_or_else(|| invalid("lineage parent was not pinned or core-created"))?;
        if !same(identity, expected) {
            return Err(invalid("lineage parent identity mismatch"));
        }
        Ok(())
    }
    fn expect(
        &self,
        path: &Path,
        old: Option<&Identity>,
        allow_untracked_absence: bool,
    ) -> io::Result<()> {
        match self.nodes.get(path) {
            Some(node) if equal(node.identity.as_ref(), old) => Ok(()),
            None if old.is_none() && allow_untracked_absence => Ok(()),
            _ => Err(invalid(
                "owned effect does not continue the recorded old identity",
            )),
        }
    }
    fn proof(
        j: &Journal,
        path: &Path,
        old: Option<&Identity>,
        new: Option<&Identity>,
    ) -> io::Result<()> {
        let path = HostPath::new(path)?;
        if !j
            .changes
            .iter()
            .any(|c| c.path == path && equal(c.old.as_ref(), old) && equal(c.new.as_ref(), new))
        {
            return Err(invalid(
                "completed effect lacks its exact old/new observation",
            ));
        }
        Ok(())
    }
    fn apply(&mut self, j: &Journal, i: &EffectIntent) -> io::Result<()> {
        if !i.completed || i.cancelled {
            return Err(invalid(
                "incomplete or cancelled intent cannot advance receipt lineage",
            ));
        }
        let to = i.to.get()?;
        if self.touched.len().saturating_add(3) > MAX_NODES {
            return Err(limit());
        }
        self.touched.insert(to.clone());
        if let Some(from) = &i.from {
            self.touched.insert(from.get()?);
        }
        if let Some(prior) = &i.prior {
            self.touched.insert(prior.get()?);
        }
        self.parent(&to, &i.to_parent)?;
        if i.kind != EffectKind::Stage
            && !j.footprint.iter().any(|p| {
                p.get()
                    .is_ok_and(|p| to == p || (j.plan != "control-files" && to.starts_with(&p)))
            })
        {
            return Err(invalid("effect exceeds journal footprint"));
        }
        if (i.kind == EffectKind::Adopt && j.plan != "adopt")
            || (i.kind == EffectKind::Create && j.plan == "control-files")
        {
            return Err(invalid("effect kind does not match its plan"));
        }
        match i.kind {
            EffectKind::Create | EffectKind::Stage => {
                if i.old.is_some() || i.from.is_some() || i.prior.is_some() {
                    return Err(invalid("invalid creation lineage"));
                }
                let new = i
                    .new
                    .as_ref()
                    .ok_or_else(|| invalid("creation has no observed identity"))?;
                self.expect(&to, None, true)?;
                if i.kind == EffectKind::Create {
                    Self::proof(j, &to, None, Some(new))?;
                } else if !j.artifacts.contains(&i.to)
                    || !to
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&format!(".iii-stage-{}-", j.id)))
                {
                    return Err(invalid("stage not bound to originating journal"));
                }
                self.put(
                    to,
                    Node {
                        identity: Some(new.clone()),
                        owned: true,
                    },
                )?;
            }
            EffectKind::Publish => {
                let from = i
                    .from
                    .as_ref()
                    .ok_or_else(|| invalid("publication has no stage"))?
                    .get()?;
                self.parent(
                    &from,
                    i.from_parent
                        .as_ref()
                        .ok_or_else(|| invalid("stage parent proof missing"))?,
                )?;
                let new = i
                    .new
                    .as_ref()
                    .ok_or_else(|| invalid("publication identity missing"))?;
                // File-only replacement here; arbitrary subtree re-clone is not
                // implicitly authorized by a receipt for a previously retired tree.
                if new.mode & libc::S_IFMT != libc::S_IFREG
                    || i.old
                        .as_ref()
                        .is_some_and(|id| id.mode & libc::S_IFMT != libc::S_IFREG)
                {
                    return Err(invalid("unsupported tree replacement in receipt lineage"));
                }
                if !j.intents.iter().any(|stage| {
                    stage.kind == EffectKind::Stage
                        && stage.to.get().is_ok_and(|p| p == from)
                        && stage.completed
                        && !stage.cancelled
                        && equal(stage.new.as_ref(), Some(new))
                }) {
                    return Err(invalid("publication stage is not owned by this journal"));
                }
                self.expect(&from, Some(new), false)?;
                self.expect(&to, i.old.as_ref(), true)?;
                let owned = i.old.is_none() || self.nodes.get(&to).is_some_and(|n| n.owned);
                if let Some(old) = &i.old {
                    let prior = i
                        .prior
                        .as_ref()
                        .ok_or_else(|| invalid("prior preservation missing"))?
                        .get()?;
                    if !j
                        .artifacts
                        .contains(i.prior.as_ref().ok_or_else(|| invalid("missing prior"))?)
                        || prior.parent() != to.parent()
                    {
                        return Err(invalid("prior changed parent"));
                    }
                    self.expect(&prior, None, true)?;
                    Self::proof(j, &prior, None, Some(old))?;
                    self.put(
                        prior,
                        Node {
                            identity: Some(old.clone()),
                            owned: false,
                        },
                    )?;
                } else if i.prior.is_some() {
                    return Err(invalid("unexpected prior for absent target"));
                }
                Self::proof(j, &from, Some(new), None)?;
                Self::proof(j, &to, i.old.as_ref(), Some(new))?;
                self.put(
                    from,
                    Node {
                        identity: None,
                        owned: true,
                    },
                )?;
                self.put(
                    to,
                    Node {
                        identity: Some(new.clone()),
                        owned,
                    },
                )?;
            }
            EffectKind::Adopt => {
                if i.old.is_some() || i.prior.is_some() {
                    return Err(invalid("Adopt destination must be absent"));
                }
                let from = i
                    .from
                    .as_ref()
                    .ok_or_else(|| invalid("Adopt source missing"))?
                    .get()?;
                if !j
                    .footprint
                    .iter()
                    .any(|p| p.get().is_ok_and(|p| from.starts_with(p)))
                {
                    return Err(invalid("Adopt source exceeds footprint"));
                }
                if overlaps(&from, &to) {
                    return Err(invalid("overlapping Adopt lineage"));
                }
                self.parent(
                    &from,
                    i.from_parent
                        .as_ref()
                        .ok_or_else(|| invalid("Adopt source parent missing"))?,
                )?;
                let identity = i
                    .new
                    .as_ref()
                    .ok_or_else(|| invalid("Adopt identity missing"))?;
                self.expect(&from, Some(identity), false)?;
                self.expect(&to, None, true)?;
                Self::proof(j, &from, Some(identity), None)?;
                Self::proof(j, &to, None, Some(identity))?;
                let moved: Vec<_> = self
                    .nodes
                    .iter()
                    .filter(|(p, _)| p.starts_with(&from))
                    .map(|(p, n)| (p.clone(), n.clone()))
                    .collect();
                for (path, node) in moved {
                    let destination = to.join(path.strip_prefix(&from).map_err(io::Error::other)?);
                    self.expect(&destination, None, true)?;
                    if path != from && node.identity.is_some() {
                        Self::proof(j, &path, node.identity.as_ref(), None)?;
                        Self::proof(j, &destination, None, node.identity.as_ref())?;
                    }
                    self.put(
                        path,
                        Node {
                            identity: None,
                            owned: node.owned,
                        },
                    )?;
                    self.put(destination, node)?;
                }
            }
            EffectKind::Diagnostic | EffectKind::Remove => {
                return Err(invalid(
                    "effect kind cannot advance this retirement lineage",
                ));
            }
        }
        Ok(())
    }
}
fn pins_equal(a: &Entry, b: &Entry) -> bool {
    a.path == b.path
        && a.role == b.role
        && a.pins.len() == b.pins.len()
        && a.pins
            .iter()
            .all(|(p, id)| b.pins.iter().any(|(q, other)| p == q && same(id, other)))
}
/// Validates original declarations plus owned journal lineage. All reads of
/// current paths happen only AFTER replay, to check its result (not to grant it).
pub(super) fn validate(
    anchor: &Anchor,
    original: &[Entry],
    op: &Operation,
    targets: &[PathBuf],
    never_spawned: bool,
) -> io::Result<Vec<Entry>> {
    if original.len() > super::catalog::MAX_PATHS
        || targets.len() > super::catalog::MAX_PATHS
        || op.journals.len() > 256
    {
        return Err(limit());
    }
    if op.epoch < anchor.epoch {
        return Err(invalid("operation epoch moved backwards"));
    }
    for (id, epoch, digest) in &anchor.committed {
        let j = op
            .journals
            .iter()
            .find(|j| &j.id == id && j.epoch == *epoch && j.committed)
            .ok_or_else(|| invalid("receipt journal history disappeared"))?;
        if super::recovery::digest(j)? != *digest {
            return Err(invalid("receipt journal history changed"));
        }
    }
    let mut replay = Replay::initial(original, anchor)?;
    // Walk dependencies backwards: an Adopt may consume a source whose earlier
    // ControlFiles publication is not itself under the eventual remove target.
    // Disjoint post-retirement work is not a reason to expand removal authority.
    let mut needed = targets.to_vec();
    let mut selected = BTreeSet::new();
    for journal in op.journals.iter().rev() {
        if anchor.committed.iter().any(|(id, _, _)| id == &journal.id) {
            continue;
        }
        let footprint = journal
            .footprint
            .iter()
            .map(|p| p.get())
            .collect::<io::Result<Vec<_>>>()?;
        if footprint
            .iter()
            .any(|p| needed.iter().any(|t| overlaps(p, t)))
        {
            selected.insert(journal.id.clone());
            for path in footprint {
                if !needed.contains(&path) {
                    if needed.len() >= MAX_NODES {
                        return Err(limit());
                    }
                    needed.push(path);
                }
            }
        }
    }
    let mut previous = 0;
    for journal in &op.journals {
        if !journal.committed || journal.epoch <= previous || journal.epoch > op.epoch {
            return Err(invalid("unfinished or unordered operation journals"));
        }
        previous = journal.epoch;
        if anchor.committed.iter().any(|(id, _, _)| id == &journal.id) {
            continue;
        }
        if journal.epoch <= anchor.epoch
            && !anchor
                .pending
                .iter()
                .any(|(id, epoch)| id == &journal.id && *epoch == journal.epoch)
        {
            return Err(invalid(
                "unrecognized old journal inserted into receipt history",
            ));
        }
        if !selected.contains(&journal.id) {
            continue;
        }
        if journal.intents.is_empty()
            && journal.changes.is_empty()
            && journal.disposals.is_empty()
            && journal.permissions.is_empty()
        {
            continue;
        }
        if !["control-files", "adopt", "create"].contains(&journal.plan.as_str())
            || !journal.permissions.is_empty()
        {
            return Err(invalid("unsupported mutation of retired receipt target"));
        }
        if journal.disposals.iter().any(|d| !d.discarded) {
            return Err(invalid("unresolved disposal cannot advance lineage"));
        }
        for intent in &journal.intents {
            replay.apply(journal, intent)?;
        }
        // Disposal is not publication authority; only consume an owned stage
        // with its exact completed discard evidence, never a target replacement.
        for disposal in &journal.disposals {
            let path = disposal.path.get()?;
            replay.parent(&path, &disposal.parent)?;
            replay.expect(&path, Some(&disposal.file), false)?;
            if !journal.intents.iter().any(|i| {
                i.kind == EffectKind::Stage
                    && i.to == disposal.path
                    && equal(i.new.as_ref(), Some(&disposal.file))
            }) {
                return Err(invalid("foreign disposal in lineage"));
            }
            Replay::proof(journal, &path, Some(&disposal.file), None)?;
            replay.put(
                path,
                Node {
                    identity: None,
                    owned: true,
                },
            )?;
        }
    }
    // Retention is part of the proof too. Validate the last derived placement
    // of touched/prior paths, without turning them into allowed Remove targets.
    for path in &replay.touched {
        let mut pins = Vec::new();
        for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            if let Some(id) = replay.nodes.get(ancestor).and_then(|n| n.identity.as_ref()) {
                pins.push((HostPath::new(ancestor)?, id.clone()));
            }
        }
        let entry = Entry {
            path: HostPath::new(path)?,
            role: PathRole::MutableTree,
            pins,
        };
        super::validate_entry(&entry)?;
    }
    let mut result = Vec::new();
    for target in targets {
        let initial = original
            .iter()
            .find(|e| {
                matches!(e.role, PathRole::MutableTree | PathRole::MutableFile)
                    && e.path.get().is_ok_and(|p| p == *target)
            })
            .ok_or_else(|| invalid("target absent from original exact receipt declaration"))?;
        let current = op
            .entries
            .iter()
            .find(|e| e.path == initial.path)
            .ok_or_else(|| invalid("receipt target declaration disappeared"))?;
        let mut expected = Entry {
            path: initial.path.clone(),
            role: initial.role,
            pins: Vec::new(),
        };
        for ancestor in target.ancestors().collect::<Vec<_>>().into_iter().rev() {
            if let Some(identity) = replay.nodes.get(ancestor).and_then(|n| n.identity.as_ref()) {
                expected
                    .pins
                    .push((HostPath::new(ancestor)?, identity.clone()));
            }
        }
        if !pins_equal(&expected, current) {
            return Err(invalid(
                "current target pins are not derived from the owned lineage",
            ));
        }
        if never_spawned && !replay.nodes.get(target).is_some_and(|n| n.owned) {
            return Err(invalid(
                "never-spawned receipt cannot delete preexisting or legacy data",
            ));
        }
        super::validate_entry(&expected)?;
        result.push(expected);
    }
    Ok(result)
}
