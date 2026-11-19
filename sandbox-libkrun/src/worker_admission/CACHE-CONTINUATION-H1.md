# H1: cache continuation from the current writer-retirement capability

Incremental SOURCE implementation on `d5fd9971c7066bb81af5a1ad2e3e5d9e3d09fc50`
(tree `8836a6ef9411d2face190a548c1fd734d86ba928`). H1 contract SHA:
`c4bdd900c451eb13c1616f744a91981142b8312950ec36fe89bfd7cd8220c954`.
No compiler, Rust test, process/VM/socket, filesystem fixture, network or host
provisioning ran. This is not a claim that the combined WorkerVms paths pass.

## Additive public-to-crate API; all H/F/G signatures unchanged

```rust
impl WriterRetirement {
    pub(crate) fn probe_cache(
        &self, lease: &Lease, entries: &[CacheEntry<'_>],
    ) -> io::Result<CacheProbe>;

    pub(crate) fn prepare_cache(
        &self, lease: &Lease, entries: &[CacheEntry<'_>],
    ) -> io::Result<CachePreparation>;
}
```

The returned CacheProbe/CachePreparation are exactly the existing H types. Their
read/checkpoint/cancel/prepare/guard/handoff methods and bounds are unchanged.
The existing `Lease::probe_cache/prepare_cache` STILL reject runtime history:
only the private actual WriterRetirement can issue the continuation. No analogous
NeverSpawned method, bool/PID/ExitStatus/status-map constructor, generic release,
role change or new lifecycle is introduced.

## Required state before issuance

Use the SAME original lifecycle Lease Arc, not another context import with the
same operation ID. The real receipt must be unused by Remove and still represent
the latest actual retired writer. Current runtime ID/epoch AND the immutable
terminal runtime history anchored at receipt issuance must match. A later writer
invalidates the receipt even if that newer runtime is then Retired/Cancelled.
Every runtime must be clean Retired with actual imported-holder evidence or a
legitimate never-activated Cancelled record; no guest-consistency assertion is
made. `guest_consistent` remains false.

Finish host writer/pump guards, exact G endpoint settlement, diagnostic handles,
OwnerRead ACL restoration and mutation journals BEFORE requesting continuation.
No imported holder, Read/Mutation/OwnerRead/Endpoint hold, unfinished journal,
uncertain/corrupt operation or unfinished recovery may remain. Retired endpoints
must have exact owned records, not inferred absence. Foreign operation/sentinel,
changed pins, an introduced mutable layout declaration or old Strict identity
change refuses. No current-path observation grants replacement authority.

F's existing lineage validator verifies legitimate same-operation final
ControlFiles/Adopt effects against original receipt entries and journals. Private
journal-generated stage/prior evidence may remain; it is not an application
layout declaration. Cache additions are proved Strict transfer entries only.
An existing receipt does not gain permission to remove those added paths.

## Capability binding and revocation

A private, non-serializable CacheContinuation retains the real WriterRetirement
and original Lease through probe, preparation clones and nested guards. Its
serialized cache binding records runtime ID/epoch plus digests of runtime history
and the consumer's current entries/journals/endpoints/holders/recovery/cache proofs.
Those digests are integrity evidence, NOT a deserializable issuer. Catalog creation
refuses continuation metadata without the live private capability.

Issuance and transfer take locks in the same order as Remove/G settlement:
private receipt gate, then the nonblocking catalog transaction. Runtime creation
and mutation issuance serialize through that catalog. No callback, async wait,
diagnostic-lock wait, pull or image writer runs under the private gate/catalog
mutex. Guards keep the capability and revalidate it at issuance/checkpoint,
including after waiting for a diagnostic lock.

The exact consumer snapshot is rechecked immediately before the ONE-transaction
handoff. This appends/deduplicates only proved Strict entries, records H's two-sided
transfer evidence and retires only the preparation. It does not change lifecycle
identity, runtimes/journals, old cache claims or Remove's one-shot flag. A concurrent
Remove cannot consume the receipt midway through the handoff; a concurrent new
runtime is ordered before it (refusal) or after it (reads already protected).
A consumer change after continuation issuance, even a later completed no-effect
mutation, invalidates that issued handle. Request a new continuation from the
still-current receipt if appropriate; do not refresh the old handle. Failure leaves
preparation evidence fenced; ordinary H cancellation/save semantics remain.

## Caller example: A -> new B reference -> B runtime

Illustrative caller helper names below are not core implementations. B owns the
actual OCI selection, S102/auth/instance/retention policy and writer joins.

```rust
// The original admission remains stored per state_key. Runtime A really exited.
let completion_a = child_a.wait().await;
let receipt_a = completion_a.retirement().map_err(RuntimeFailure::to_io_error)?;
finish_host_writers_and_logs().await?;
receipt_a.retire_runtime_endpoints(&admission, &runtime_a_endpoints)?;
retain_with_owned_adopt_if_required(&admission)?;

// A future build may name a different plausible base image. Declare only its
// exact deduplicated H candidates, not the cache parent or mutable consumer paths.
let candidates = exact_cache_candidates(new_reference, cache_parent)?;
let probe = receipt_a.probe_cache(&admission, &candidates)?;
let selection = validate_reference_population_digest_freshness(&probe, new_reference)?;
let admission_again = match selection {
    Warm(selected) => {
        // All provisional UseGuards must be dropped before handoff.
        probe.handoff(&admission, &selected)?
    }
    Miss => {
        let prep = probe.prepare()?;
        let selected = recheck_selection_under_exclusive_prep(&prep, new_reference)?;
        pull_publish_root_digest_if_needed(&prep, &selected).await?;
        build_erofs_restore_acl_discard_spool_finish_diagnostics(&prep, &selected)?;
        prep.handoff(&admission, &selected)?
    }
};
// admission_again is a clone of the SAME Arc, never a replacement lifecycle.
let child_b = admission_again.prepare_runtime(&runtime_b_paths)?.spawn(command_b)?;
// receipt_a is stale as soon as runtime B exists, even if B is then cancelled.
let completion_b = child_b.wait().await;
let receipt_b = completion_b.retirement().map_err(RuntimeFailure::to_io_error)?;
```

Direct `receipt.prepare_cache` is also provided when a caller has already selected
the genuine miss/rebuild branch; it still conflicts with every protected reader.
Do not populate `MutationPlan::Cache { root: cache_parent }` on the lifecycle.
The root parent cold-creation flow and exact preparation slots remain as H.

Same-reference reuse should normally use B's stored prepared result and existing
Strict guards, not another transfer. A warm probe with partially overlapping
canonical/pinned candidates can reuse exact already-Strict facts only if their
pin/type/path binding is identical to a previous cache handoff record. This is
useful when new reference B shares a candidate with A but selects a disjoint
already-populated pinned candidate. It is not a writer upgrade.

## Supported/blocked overlap matrix

| Request on the original retired lifecycle | Result |
| --- | --- |
| New disjoint warm B source/lower/digest | Provisional Strict probe, semantic caller check, atomic Strict append |
| New disjoint cold B | Separate exact prep; recheck, guards/ACL/FD completion, atomic handoff |
| Exact old Strict candidate with unchanged proved cache binding | Warm probe reuse/dedup allowed; no role or pin change |
| Old exact root shared plus new disjoint populated pinned candidate | Warm read set may reuse proved old facts and append new facts |
| Same prepared result with no new selection | Caller reuses existing Strict claims/RuntimeWriter reads |
| Rebuild/OwnerRead/chmod/write touching any old Strict cache claim | Busy; old claims are NOT withdrawn to serve a changed tag |
| Ancestor/descendant overlap, unproved ordinary Strict path, changed inode | Refuse; no generic matching or canonicalization bypass |
| New reference while old runtime/read/host mutation/endpoint remains active | Refuse until actual retirement and exact work completion |
| Issued handle after a new runtime, consumer mutation/import, used receipt | Stale/refuse; no refresh or one-shot reset |
| New mutable layout declaration after original receipt | Refuse; cache continuation cannot bless it |

F Remove is still limited to ORIGINAL exact mutable layout targets, following F
journal lineage. Adding B Strict claims never makes B removable. A successful H1
append does not consume the receipt, so existing layout cleanup can still use it
until a subsequent runtime or Remove itself invalidates it. Protected old reads
remain independent and continue excluding third-party writers; unrelated images
can be prepared without a whole-cache lifetime lock.

## Authored evidence and unchanged boundaries

All148 previous test-entry names remain. Current168 entries comprise167 scenarios
and the existing cross-exec helper (20 added H1 scenarios). New cases cover actual owned wait A,
G endpoint settlement, warm/cold B on the same Arc, runtime B consumption and new
actual retirement; identical cache selection reuse and partial candidate overlap;
old read exclusion, foreign/used/stale receipt, imported holder and host reads,
OwnerRead/journal/recovery/endpoint gates, consumer mutation, runtime already active
or later cancelled/retired, guard/diagnostic revocation, canceled probe/abandoned
prep, before/after/torn transfer faults and catalog-ordered Remove/runtime races.
The metadata-only negative uses no fake successful authority constructor. Positive
activation uses existing real OFD-import fixtures; the existing cross-exec G suite
is preserved. Every fixture is AUTHORED_NOTRUN, not process/kernel acceptance.

Source checks: rustfmt and rustfmt --check on eight edited Rust files exit0;
git diff --check exit0; bounded Python exact-base/11-owned-path/prior-test-name/
public-signature/private-capability checks exit0. Runtime, F lineage, pidfd,
endpoint, storage, recovery and earlier standalone test files are byte-identical
to base. No compilation or fixture execution occurred. The source-inspection fix
that dropped a test catalog transaction before checking receipt refusal prevents
mistaking catalog Busy for the intended consumed-receipt negative.

No caller, CLI/main/admin/root README/HOST-ADMISSION/Cargo/dependency or other
worktree was changed. Caller source was read only through immutable Gitd5fd9971.
B may implement the first-consumer H routes and all36 fixtures independently, then
wire these exact H1 methods on the delivered immutable commit. Same-reference,
new-reference repeat-build, root0000 non-overlay clone ordering, positive stop_all,
platform/kernel/ACL, performance, cross-exec descendants, legacy/restart/prune/
control-parent cleanup and authenticated migration8x2 remain explicit combined
validation gates. One admitted test workload at a time, one independent integrated
review and one Workers PR; no source result grants host setup or remote actions.
