# H: exact shared cache with one-transaction read handoff

SOURCE candidate on consolidated base `099a9d128aee96e28f9d7db72f446dbaec64aab7`,
tree `a493d0039e4f06c5f0d43826ee2a8ee338612e99`. Source contract SHA:
`3c532b6b9fd2e6986ea78776def2f201551654e3a4905858d23ee00f75339cae`.
No caller, CLI/admin, Cargo/dependency, cache payload or host configuration changed.
No Rust compilation or process/filesystem/kernel fixture was executed.

## Exact crate-internal interface (real implementation, not a stub)

```rust
pub(crate) struct CacheEntry<'a> { pub root: &'a Path }
impl Lease {
    pub(crate) fn probe_cache(&self, entries: &[CacheEntry<'_>])
        -> io::Result<CacheProbe>;
    pub(crate) fn prepare_cache(&self, entries: &[CacheEntry<'_>])
        -> io::Result<CachePreparation>;
}
impl CacheProbe { // not Clone, provisional only
    pub(crate) fn exists(&self, path: &Path) -> io::Result<bool>;
    pub(crate) fn use_path(&self, path: &Path) -> io::Result<UseGuard>;
    pub(crate) fn checkpoint(&self) -> io::Result<()>;
    pub(crate) fn handoff(self, consumer: &Lease, selected_root: &Path)
        -> io::Result<Lease>;
    pub(crate) fn prepare(self) -> io::Result<CachePreparation>;
    pub(crate) fn cancel(self) -> io::Result<()>;
}
#[derive(Clone)] pub(crate) struct CachePreparation;
impl CachePreparation {
    pub(crate) fn checkpoint(&self) -> io::Result<()>;
    pub(crate) fn exists(&self, path: &Path) -> io::Result<bool>;
    pub(crate) fn use_path(&self, path: &Path) -> io::Result<UseGuard>;
    pub(crate) fn begin_entry(&self, root: &Path) -> io::Result<MutationGuard>;
    pub(crate) fn begin_erofs(&self, root: &Path) -> io::Result<MutationGuard>;
    pub(crate) fn owner_read(&self, root: &Path) -> io::Result<OwnerReadGuard>;
    pub(crate) fn handoff(&self, consumer: &Lease, selected_root: &Path)
        -> io::Result<Lease>;
}
```

CacheEntry roots must be absolute, distinct and nonoverlapping; deduplicate the
canonical and pinned slug if they are the same. Maximum4 candidates. Each root
binds only that exact source tree plus the existing canonical sidecars `.erofs`,
`.digest`, `.lock`, `.erofs.lock`, `.erofs.spool`. No caller-chosen suffix expands
this set. Every parent must already exist and be owned/protected. Cold creation
of the cache parent is a separate short Create operation, committed/dropped before
acquiring cache claims. No whole-cache-parent MutableTree is retained.

The original consumer must be its original holder, pre-runtime, unmodified since
probe/preparation issuance and without overlapping cache/broad-parent claims,
open holds, imported holders, runtime or endpoint records. Prepare all unrelated
layout declarations first; obtain runtime/endpoint handles AFTER handoff. This
avoids a generic release/downgrade API and binds the transfer to one exact intended
lifecycle, namespace/sentinel/epoch/entry snapshot. A stale/foreign target refuses.

## Read-first path — no payload or diagnostic mutation

Probe is a separate operation containing Strict entries only for each source,
lower and digest. It pins present identities and absent candidates; filesystem
existence never selects the exclusion domain. `exists` and `use_path` accept only
these exact paths; read a source tree through its root guard. A warm probe has no
OwnerRead permission grant, mutation guard, writer lock, staging, chmod, mtime/LRU
write or cache diagnostic creation. Catalog admission metadata is expected.

The caller MUST check actual OCI reference/digest/pin selection and root population,
plus lower freshness, under these guards before declaring a warm hit. Core does
not authenticate OCI content or interpret a filename/hash as provenance. Digest
absence is allowed as a protected selection fact for historical unpinned hits;
references requiring a digest must refuse/miss in the caller. Existing mode0000
entries that cannot be read do not justify temporary chmod on the warm path: use
the explicit exclusive branch where needed or return the real access error.

```rust
let candidates = deduplicated_canonical_and_pinned_roots(reference, cache_parent)?;
let entries: Vec<_> = candidates.iter().map(|root| CacheEntry { root }).collect();
let probe = lifecycle.probe_cache(&entries)?;
let warm = {
    // Check every selection fact; absent candidates are fenced too.
    // Existing helper variants must read through the guards, not touch LRU.
    let selected = select_under_probe(&probe, reference)?;
    let source = probe.use_path(&selected)?;
    let lower = probe.use_path(&base_erofs_path(&selected))?;
    check_population_digest_and_freshness(source.path(), lower.path(), reference)?
}; // all UseGuards dropped before handoff or prepare
let lifecycle = if let Some(selected) = warm {
    probe.handoff(&lifecycle, &selected)?
} else {
    let prep = probe.prepare()?;
    prepare_and_handoff(prep, &lifecycle, reference)?
};
```

A live provisional UseGuard prevents `prepare`/`cancel` and handoff. A miss may
release ONLY that provisional read operation, then acquire a NEW exact exclusive
preparation and recheck selection. That intentional miss-path gap is not a gap in
published reader protection: no consumption has begun. A competing producer may
win; return Busy or re-evaluate under the actual acquired capability, never retry
into effects. `prepare_cache` also exists for a caller already committed to the
write branch; it does not bypass active readers. There is no reader-to-writer
upgrade for the long-lived consumer.

## Exact preparation using existing opaque mutation guards

```rust
let prep = probe.prepare()?;
let selected = select_again_under_exclusive_preparation(&prep, reference)?;
if needs_pull_under_preparation(&prep, &selected, reference)? {
    let mut mutation = prep.begin_entry(&selected)?;
    let mut diagnostic = mutation.diagnostic_lock(&rootfs_lock_path(&selected))?;
    diagnostic.lock_exclusive()?; // no catalog mutex is held while waiting
    diagnostic.write_pid(std::process::id())?; // diagnostic only, not authority
    // Re-evaluate selection under the existing cache protocol lock.
    let tree = mutation.stage_tree(&selected)?;
    let digests = pull_into_owned_stage_and_join_all_work(tree.path()).await?;
    let mut digest = mutation.stage_file(&digest_path(&selected), 0o600)?;
    write_digest_to_owned_fd(digest.file_mut(), &digests)?;
    mutation.publish_tree(tree)?;
    mutation.publish_file(digest)?;
    diagnostic.clear()?;
    drop(diagnostic);
    mutation.commit()?;
}
if needs_erofs_under_preparation(&prep, &selected)? {
    let mut source = prep.owner_read(&selected)?;
    let mut mutation = prep.begin_erofs(&selected)?;
    let mut diagnostic = mutation.diagnostic_lock(&erofs_lock_path(&selected))?;
    diagnostic.lock_exclusive()?;
    let mut image = mutation.stage_file(&base_erofs_path(&selected), 0o600)?;
    let mut spool = mutation.stage_file(&erofs_spool_slot(&selected), 0o600)?;
    build_joined_writer_with_owned_fds(&mut source, &mut image, &mut spool)?;
    source.finish()?; // exact E ACL restoration BEFORE publication
    mutation.discard_file(spool)?;
    mutation.publish_file(image)?;
    diagnostic.clear()?;
    drop(diagnostic);
    mutation.commit()?;
}
let lifecycle = prep.handoff(&lifecycle, &selected)?;
```

The example denotes caller helpers, not newly provided producer implementations.
`begin_entry` privately constructs a narrow source/digest/lock union, not public
parent authority: tree publication only targets the root; file publication only
its digest; the lock inode is opened through DiagnosticLock and never staged.
`begin_erofs` uses the already fixed exact output/spool plan. OwnerRead is enabled
only for this private preparation's exact source root; ManagedCacheRead remains
exclusive everywhere and is NOT a shared-read exception.

All blocking/async work must own its preparation/guard through actual completion;
no pathname writer may outlive staged-tree publication. Source tree writer handles
are caller-owned and must be joined before returning the stage, as in the existing
staging contract. For regular cache stage FDs, core adds an OFD/flock lifetime fence:
publication/discard closes the original then requires an independent nonblocking
lock, so surviving `try_clone` writer FDs refuse. This does not lock project/claim
roots. Diagnostic transfer also independently tests existing exact lock inodes;
no new file or diagnostic write is performed by that proof.

## Atomic transfer and post-transfer revocation

Handoff uses ONE catalog mutex transaction to validate the original consumer,
preparation sentinel/holder/epoch, all pins, no holds/import/runtime/endpoints,
committed journals, completed stage/disposal outcomes, restored ACL intents and
finished diagnostic ownership. It installs Strict root/lower/digest entries into
the original lifecycle and clears ONLY this cache operation's mutable interval
union, marks it closed/Transferred and writes source+consumer transfer receipts.
The selected source and lower must exist with the correct types; candidate/digest
absence remains a Strict claim, not permission to ignore a later enrollment.
No release-then-reopen window occurs on the preparation-to-reader or warm-to-reader
transfer. No broad cache entries remain to conflict merely because stale preparation
clones retain their sentinel FD. Every cache issuer consults its closed generation;
a stale clone cannot stage, chmod, create a read guard or transfer again.

Failure before persistence leaves old exclusive coverage; partial current-record
writes fail integrity validation with evidence retained, never fallback to previous.
A modeled completion error after durable transfer keeps consumer Strict coverage and
marks uncertainty. No partial cache result is returned as successful handoff.
Abandoned preparation becomes uncertain; generic recovery refuses to invent a
completed read handoff. Administrative archival still preserves completed metadata.
Consumer receipts can remove only their original mutable lifecycle paths: shared
cache entries are Strict, with no cache Remove/prune/endpoint/runtime export authority.

## Required B production routing changes

- `local_worker::admit_cache_paths` must stop adding cache parent MutableTree,
  permanent ManagedCacheRead roots and MutableFile sidecars to the lifecycle.
  Use the probe/prep interface after unrelated layout declaration, before effects.
- `rootfs_cache::ensure_rootfs_admitted_at_notify` must split pure read-first
  selection/populated/digest checks from the miss branch. It must not create/open/
  write a diagnostic or acquire Cache mutation on a valid warm hit.
- `ensure_base_erofs_admitted` must make the unchanged-image check through Strict
  guards, not OwnerRead/mutation. Actual rebuild uses prep.owner_read/begin_erofs,
  restored ACLs and explicit spool/diagnostic completion, then handoff.
- sandbox create/adapters and WorkerVms prepare/build/start carry only the returned
  consumer Lease into launch. The launcher consumes the prepared lower instead of
  invoking a writer while its own Strict claim is already active. Runtime inputs
  use the actual RuntimeWriter::use_path and retain independent reader lifetimes.
- Keep S102/cap/auth/reference selection, lower-only isolation and all36 external
  scenario oracles. Two same-cache warm lifecycles in ONE context must succeed;
  a third true rebuild may honestly be Busy. No duplicate cache/namespace workaround.

## Evidence, bounds and remaining gates

At most4 distinct candidate roots,24 preparation entries/12 Strict entries and16
cache transfers per lifecycle, plus existing catalog/stage/ACL/journal bounds.
Each transfer retains evidence within existing record limits; full/unsupported
filesystem locks refuse, never omit resources. Source warm checks create catalog
metadata only, no cache payload/lock/LRU effects. Shared readers are independent:
retiring one does not release another's exact claim; unrelated image prep can
proceed with already provisioned parents. Read claims include candidate selection
metadata and may conservatively block preparation of an alternate candidate used
by the same reference; this is explicit, not a whole-cache lock.

All127 previous test-entry names remain; current148 entries comprise147 scenario
tests and the existing cross-exec helper. The21 added H scenarios are authored
only. Tests are AUTHORED_NOTRUN: cold first handoff + second warm reader, populated warm
no-write route, independent runtime reads, unrelated image, absent pinned/digest
selection, live-probe refusal, diagnostic/OwnerRead/stage/FD completion, canceled
prep, stale clone, foreign context/epoch/import/runtime, replacement/hardlink/symlink,
claim-lock independence and before/after/torn-save failure boundaries. No core test
claims OCI provenance or a successful VM. Compile/kernel/OFD/ACL/cross-exec and
public cold/warm/multi-sandbox/performance validation await parent admission after
B routing and fan-in, followed by ONE combined independent review and Workers PR.
Source checks for this increment: rustfmt and rustfmt --check on nine edited Rust
files exit0; git diff --check exit0; bounded Python base/11-owned-path/API/127-prior-
name checks exit0. F lineage/retirement/pidfd/endpoint/storage and earlier standalone
test files are byte-identical to the consolidated base. Compiled0/executed0. An
intermediate regex edit was rejected before writing because a literal `||` was not
escaped, and an import-cleanup anchor missed formatted text; both were corrected
against the actual source without rerunning any functional workload.

No host bootstrap/cache mutation, networking, cleanup or publication was authorized
or executed by this source task. Existing F/G, lineage and migration8x2 gates remain.
