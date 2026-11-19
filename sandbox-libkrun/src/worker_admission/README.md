# Admission v2 core: runtime exit resources G source candidate

The G1/G2/G4 increment after `71e7d6660b746bf51ed6981f67185c36cba262f0`
adds exact runtime-owned read/endpoint guards and parent-side endpoint settlement
through the same private WriterRetirement. See [EXIT-RESOURCES-G.md](EXIT-RESOURCES-G.md)
for the three exact APIs, persisted holder/runtime-epoch ownership, cross-exec
fixture and integration sequence. F lineage is complete in source and unchanged;
G does not consume or expand its one-shot Remove authority. Current127 authored
test entries include126 scenarios and one cross-exec helper; compiled0/executed0.

The concrete F1/F3 consumer seam and complete spawn/control/wait/cleanup examples
are in [RETIREMENT-F.md](RETIREMENT-F.md). This increment is based on
`4202dc282f751cb8c85534f2b0dd9d332e057616`; E1/E2/E4 remain unchanged. F replaces
fallible-wait-as-reap inference with child-issued pidfd control, distinct OS-reap
observation and an opaque writer-retirement receipt. Raw `Lease::begin(Remove)`
now refuses; use the exact same-operation receipt methods. Caller routing F2 is
not included in this core source delivery.

The receipt-lineage increment after `b0b7572993d8ba65e32d80ae48b9074e1592000c`
keeps F consumer signatures unchanged. Final committed SAME-operation ControlFiles
and Adopt effects now advance the receipt's expected identities via anchored,
ordered old/new journals in the Remove-acquisition transaction. No re-observation
of current paths grants authority, no newly declared target is added, and all
foreign/stale/runtime/uncertainty/one-shot gates remain. See RETIREMENT-F.md for
exact support and bounds. Current total113 tests are AUTHORED_NOTRUN, including
all100 previous names; no compile or runtime acceptance is implied.

Incremental after `d556cda04ebb1885a35cb9b3840dfd52a26a33b3` (tree
`a9bab8b238469c4f71810bb90db695ab07a30be8`). C/D and earlier commits are preserved.
This increment addresses only E1 ACL-before-grant, E2 owned activity outcome and
E4 exact ControlFiles. E3 sandbox/caller routing belongs to Stream B. SOURCE ONLY:
no compiler, Rust test, process/socket/VM, mount, syncfs, bootstrap, networking or
administrative recovery was run. Formatting/static checks are not typecheck,
kernel support, normal-mode acceptance or migration proof. Fan-in, admitted tests,
ONE integrated independent review and ONE Workers PR remain mandatory.

## Stable catalog and original C1–C6

Production remains fixed to `/var/lib/iii/worker-admission/v1/<euid>`. Explicit
uid0 `bootstrap_for_uid` creates only this namespace through checked retained
parents; `system`/`check_installation` never provision or fall back. Context travels
with each Lease. Test context provisioning/open/recovery accepts an explicit
private own-temp root only under cfg(test), never environment/HOME/cwd overrides.
No issuer of OfflineGrant, BootAuthority or TransitionAuthority is added.

The bounded catalog registers lexical component intervals independently of which
payload paths exist. Both directions of containment conflict with transition;
new base registration cannot evade an old interval. `with_paths` extends ONE
operation atomically, `with_inputs` is Strict, clones retain the union. No data,
project or claim-directory flock exists. Claims therefore remain independent
(C1/C2), while missing mutable output/cache/state paths can be created through
owned effects without moving exclusion (C3). Registered base context identifies
whole-VM legacy markers, not arbitrary hex apps or generic receipt/compat files
(C5). Declaring a role is operational scope, not migration authority.

Strict identity pins are unchanged by generic rereads. Core-owned stage/publication,
NOREPLACE adoption, explicit discard and continuous runtime sessions have separate
receipts. Unknown/corrupt state, aliases not inventoried, old binaries and host
same-UID nonparticipants remain limitations. No hash, PID or successful OS wait
alone authenticates disk provenance or retires arbitrary writers.

## Amendment C source seams

### Exact EROFS union

`MutationPlan::Erofs { output: PathBuf, spool: PathBuf }` requires two explicitly
admitted MutableFile paths: absolute, distinct, no ancestor/descendant, no inode or
Strict alias. Both holds and the journal footprint are acquired atomically. It
cannot mutate a third sibling. Only output's explicitly admitted `.lock` sidecar
is diagnostic; it cannot alias either artifact. Sequence:

1. Stage output and the exact spool slot; writer owns/joins its FD clones.
2. `discard_file(spool)` durably records intent, unlinks only its own generated
   staging name relative to retained parent, fsyncs, records Discarded.
3. Publish output; preserve prior target under a recorded generated name.
4. Explicit diagnostic finish/drop, then commit. Failure is terminal.

Amendments A/B remain: diagnostic inode is never staged/renamed; AsFd is borrowed,
PID/clear require that handle's flock, waiting holds no catalog mutex. Commit
refuses outstanding handles/stages. Discard is not a payload delete API and does
not touch its semantic target. EffectReceipt keeps publication/disposal distinct.

### Owned endpoint, independently provable lifetime

`BoundEndpoint::{into_async,close}` and `EndpointListener::{accept,close}` replace
raw into_parts. No Clone/AsFd/AsRawFd/into_std/try_clone or raw listener accessor.
Owned close first closes the listener, then verifies/removes the exact receipt
fingerprint. Drop/failed conversion closes OS listener but conservatively retains
its endpoint sentinel. Relay must stop accepted streams/tasks and regain unique
wrapper ownership before close; connected streams are not listener clones.

Every endpoint has its own EX/OFD sentinel, epoch, durable BindIntent and Bound/
CloseIntent/Retired receipt. Restart verifies that endpoint's free sentinel, not
mixed-operation holders. Retirement removes ONLY its endpoint entry/hold/socket;
never clears operation uncertainty, data holds, journals or data pins. Live,
foreign, unrecorded, bind-gap or overlapping writer/control-parent states refuse.
The private parent must be outside payload/export scopes; new payload declarations
cannot invade it. D3 caller setup is a separate completed short operation BEFORE
main writer admission, not dropping a live writer fence.

### Explicit administrative recovery and bounded archival

`RecoveryAction::{ReconcileVerified,ArchiveCompleted}`, RecoveryRequest and
versioned bounded Serialize RecoveryReport implement the fixed inspection/recovery
seam. Both production functions require effective uid0 and fixed numeric-euid
namespace. Digest CAS is not authority. Inspection is read-only; recovery obtains
nonblocking operation, endpoint AND runtime sentinel ownership and refuses live
children, conflicting operations and unverified holders.

Before-effect intents bind parents, old/new/stage/prior identities for creation,
staging, publication, adoption, diagnostics and removal. Recovery intent is durable
before reconciliation writes. Completely proven NEW placement or entirely retained
OLD placement can settle without rollback/deleting stages; ambiguous half moves,
unknown creation identities and unrecorded artifacts remain `unresolved`. A
DiscardIntent with matching parent and absence can settle; a replacement is never
deleted. Unrestored permissions require exact original inode/mode/ACL evidence;
unknown runtime state never becomes retired through admin privilege.

ArchiveCompleted accepts only fully terminal operations and free sentinels. It
writes EXCL immutable bundle + fsync + NOREPLACE promotion, persists index/receipt,
moves verified sentinels, then removes the LIVE row. Bundles retain journals,
endpoint/runtime receipts and references to payload artifacts; payload/prior data
is never pruned. Recorded archive-boundary interruptions can resume idempotently;
unknown/torn staging refuses. No automatic completed-record deletion remains.
Corrupt current catalog is never reconstructed from `catalog.previous`. Current
and previous are integrity-wrapped, fixed-inode records; both remain evidence.

## Amendment D source seams

### D1/D2 owned continuous writer session

`RuntimePathKind::{GuestTree,UpperImage,ActivityStamp}`, RuntimePath,
`Lease::prepare_runtime`, `RuntimeLaunch::spawn`, RuntimeChild pipe/id/wait,
`import_runtime_writer`, RuntimeWriter clone/checkpoint/activity and RuntimeActivity::touch_at exist.
Exact declared roles/types/owner/ancestry are checked; GuestTree roots and upper
files must already exist. A missing exact ActivityStamp is EXCL-created only after
Preparing record publication; a crash gap remains unresolved. Root and protected
ancestor identity never refresh. GuestTree descendants and UpperImage bytes may
change only within the active session. Strict/readonly overlap refuses.

Spawn owns the actual Tokio Child and passes one selected CLOEXEC duplicate via
`--runtime-write-fd`; only child pre_exec clears CLOEXEC. The separate admission FD
continues the original operation. Import verifies the runtime OFD, exact namespace,
operation and an independently named imported admission holder before activation.
No raw Child/from_pid/from_status/bool reaped API exists. RuntimeChild.wait awaits
the real child, closes the parent's runtime duplicate, then acquires an independent
verified sentinel nonblocking: a descendant-held descriptor refuses retirement.
It settles only that runtime hold and its exact imported holder (including `_exit`
which runs no Drop), never other child holders. Unknown spawn/wait/cancel retains
uncertainty. Receipt says writer-retired, explicitly NOT guest/application-consistent.
Actual descendants must retain this same sentinel; no arbitrary signal permission.

### E2 owned activity outcome (supersedes raw File precheck seam)

`RuntimeWriter::activity(path) -> RuntimeActivity` opens the exact declared
ActivityStamp-kind inode, verified owner/type/nlink/parent and record binding.
RuntimeActivity is cloneable but has NO File/AsFd/try_clone escape. `touch_at(when)`
owns terminal precheck, one durable Pending slot, actual retained-FD set_modified,
and durable Clean/Failed resolution. The admitted `open_activity_file` escape was
removed; callers must capture RuntimeActivity in iii-network::ActivityStamp's
`from_updater` closure, never fall back to an independent raw-file constructor.

All activity handles and writer clones share one mutex and typed first-failure
latch. Order is activity mutex -> short catalog transaction; the catalog mutex is
released before set_modified and there are no user callbacks under either lock.
A reusable sequence/path/phase/failure slot is bounded: no journal row per packet.
The first syscall/checkpoint/persistence error is terminal. A failure to persist
failure after an effect leaves durable Pending; crash between effect and outcome
also stays Pending. RuntimeChild::wait checks this slot before clean retirement.

Additionally, the exact inherited sentinel OFD's unused cursor is a sticky local
failure mark (catalog uses positional I/O only). The core-owned parent checks it
BEFORE releasing its duplicate, so even a pre-Pending refusal during catalog Busy
survives child `_exit` until the actual waiter. It is not PID authority, not a
replacement for durable Pending/Failed, and never unlocks the shared OFD. Failed
RuntimeWriter Drop conservatively retains that descriptor. Outcome failure never
clears a prior error; payload bytes are unchanged and replacement paths receive no
write. These are source invariants; actual parent/child/crash behavior is NOTRUN.

### D4 managed cache owner read

ManagedCacheRead is distinct from Strict/MutableTree and requires an exact explicit
cache-selection declaration. OwnerReadGuard registers an exclusive hold, inventories
owned no-follow objects, preserves ORIGINAL Metadata, counts internal hardlinks
against nlink and refuses external links. Directory traversal may need temporary
owner r/x before enumeration; no file mode grant occurs before link inventory.
Only owner read/traverse bits change. Original ctime cannot be restored and is
recorded as changed, never claimed byte-identical.

### E1 exact ACL evidence BEFORE grant

The coordinator verified Linux v6.13 UAPI/source: empty-path getxattrat through
an O_PATH FD is NOT assumed usable. The implementation uses a retained, verified
accessible PARENT directory FD, one exact leaf and AT_SYMLINK_NOFOLLOW; the child
is independently pinned by O_PATH and compared before/after bounded ACL size/read.
A repr(C, align(8)) xattr_args has u64 value/u32 size/u32 flags and compile-time
size16/alignment8 checks. Only libc::SYS_getxattrat is used, guarded to Linux
x86_64/aarch64; no guessed syscall number, header installation or procfd path.
The existing locked libc version is0.2.191; compilation remains a coordinator gate.

Original system.posix_acl_access and directory system.posix_acl_default presence
AND bytes are retained in PermissionIntent with acl_captured/acl_before_grant=true
and fsynced BEFORE fchmodat2. Recheck exact ACL bytes after intent persistence and
before chmod. ENODATA alone means absent; ENOTSUP/ENOSYS/EINVAL/EBADF/EACCES and
ERANGE/size races are errors BEFORE grant, not an absent-ACL fallback. Require
actual Linux6.13 getxattrat + fchmodat2 semantics; no uname check or root privilege
substitutes for runtime capability tests. Existing accessible paths can avoid a
permission change; inaccessible parents outside the managed tree still refuse.

Post-grant comparison uses saved ORIGINAL ACL bytes and the narrowly permitted
owner-bit delta; it never reconstructs an alleged original from post-grant bytes.
Restore child-before-parent using retained inode and saved mode, then compare full
access/default ACL bytes again through the same parent-relative helper. Group,
mask/named/default entries cannot change unnoticed. Each inode also retains its
parent descriptor; FD budget is therefore up to two descriptors per4096 objects.
Internal hardlinks share one original/restoration; external hardlinks refuse.
Recovery refuses old acl_before_grant=false or acl_captured=false intents rather
than blessing them. Incomplete failures remain fenced, with no pathname restore.
Returned read File lifetime remains a caller obligation: finish follows closure
of spool/read users, not a PID/FD-count guess.

### E4 exact control files, not VM-directory authority

`MutationPlan::ControlFiles { files }` is a nonempty bounded set of absolute,
unique, nonoverlapping MutableFile declarations. Existing inode aliases, strict
aliases, unsafe parent, directory targets and wildcard/third siblings refuse.
Holds for the complete set are acquired atomically; only those files may be staged
and published. No MutableTree parent or directory creation is inferred. Thus
vm_dir/console.log and console.log.1 can be updated while a disjoint GuestTree
vm_dir/root is active, without dropping runtime fences or weakening its pins.

The existing opaque stage/publication/prior/journal/commit machinery is reused.
Partial multi-publication returns error/uncertainty; callers must not update log
cursor/len/base or switch handles until commit succeeds. Tests cover both publish
boundaries, foreign replacement and disjoint active runtime; actual console JSONL,
rotation/cursor correctness and copy-entire-log-per-append cost are caller/fan-in
gates. No in-place write or unbounded-pruning optimization was introduced.

### D5 exact authenticated normal removal

`MutationPlan::Remove { root, targets }` and remove_path admit only exact planned
normal targets. Caller retains auth, busy, instance and prune policy; core does not
infer prune. Retention/adoption must happen FIRST. Removal rejects custody,
generation, backup, `.iii-stage-*`, `.iii-prior-*`, coordination namespace and
external-hardlinked objects. A complete bounded scan precedes destruction;
statx mount IDs plus no-follow retained dirfds reject mount crossings. Each unlink/
rmdir has durable parent/identity intent and parent fsync before completion.
Nested symlinks are removed as links, never followed. Unexplained disappearance,
replacement or partial errors poison the operation. Recovery never invents prune
permission. Large trees beyond256 entries/64 depth refuse before first deletion;
this is a documented support limit, not successful destruction by omission.

## Resource bounds and evidence

256 LIVE operations/bases/endpoints/runtime sessions;256 intervals/holds/journals,
8192 pins/op;128 components/4096 bytes per path;512 effects and256 artifact/disposal
records per journal;4096 owner-read objects/permission intents;64 runtime paths.
Staged-tree sync4096 entries/depth64; removal256 entries/depth64. Catalog envelope
4MiB/payload2MiB. Archives512 operations and128MiB metadata, explicit full error,
never silent prune. Archive scans/validation are bounded but cost grows to that
limit; unsupported fchmodat2/statx/OFD/NOREPLACE fails closed, never counted as PASS.
Owner-read retains child+parent FDs per object; deployment must admit that FD budget.

All73 previous test names remain (API-migrated where E replaced the beacon
escape); total89 tests are AUTHORED_NOTRUN. E adds ACL UAPI/0000/default/named-entry
fixtures, pregrant syscall/read/journal faults and replacement refusal; actual
mtime-syscall failure, bounded outcomes, clone/thread/transient-failure controls;
and exact ControlFiles concurrent with an active GuestTree plus publish faults.
Unprivileged ACL fixtures refuse to masquerade under uid0. No test was compiled
or run; ENOSYS/unsupported filesystem is not a skipped PASS.

The two prior source seams (post-write beacon outcome and ACL-before-grant evidence)
are implemented, not an assertion of complete caller routing. Remaining gates:
libc target-symbol/typecheck, Linux6.13+ syscall/ACL filesystem matrix, positive
cross-exec `_exit`/descendant lifetime, B sandbox launch/reap bookkeeping, beacon
from_updater wiring, console cost/cursor behavior and full normal cold/warm/rebuild/
retain/prune campaigns. RuntimeChild::wait now returns RuntimeCompletion with independent os_status and
retirement results; RuntimeControl publishes actual reap before catalog work.
No forged ExitStatus or PID-based retirement API was added.

Coordinator owns CLI/admin/install wiring and caller operation reuse; no caller,
Cargo/dependency, compat_generation, auth/wire or root-document edits here.
G1/G2/G3 and A4 untouched. Actual migration still requires OS-peer provenance,
old writer/credential retirement, sanitization/profile, authorized CURRENT handoff
and8×2 guest campaigns. Source implementation does not authorize bootstrap on this
host, release, Registry or remote Git actions.
