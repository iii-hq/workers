# Admission v2 core: amendments C and D source candidate

Incremental after `ec1a2fd785b35396f9366b6d22e81446f86cd8c0` (tree
`3c479a715a240ce3691566daf84c4979feb7b6ee`), itself on consolidated `4dc14174`.
Prior implementation/review results remain historical, not acceptance. This is
SOURCE ONLY: no compiler, Rust test, process/socket/VM, mount, syncfs, bootstrap,
network, cleanup or administrative recovery was executed. Formatting/static checks
are not typecheck or runtime proof. The complete caller/core candidate still needs
fan-in, admitted tests and ONE independent integrated review/Workers PR.

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
`import_runtime_writer`, RuntimeWriter clone/checkpoint/open_activity_file exist.
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

Activity file is exact, owned, regular nlink1, no-follow and nontruncating. The
callback must retain a RuntimeWriter clone and fail terminally. **Residual seam:**
`open_activity_file -> File` plus a BEFORE-write callback cannot notify core of a
later FD write failure by itself. The consumer must retain that failure in a
shared terminal checkpoint visible to finalization; if current caller API cannot,
minimum amendment is a narrow `RuntimeWriter::record_activity_failure` or owned
activity handle. No broad writer refresh is proposed or implemented. Until this
seam is demonstrated, a clean beacon-retirement claim is not justified.

### D4 managed cache owner read

ManagedCacheRead is distinct from Strict/MutableTree and requires an exact explicit
cache-selection declaration. OwnerReadGuard registers an exclusive hold, inventories
owned no-follow objects, preserves ORIGINAL Metadata, counts internal hardlinks
against nlink and refuses external links. Directory traversal may need temporary
owner r/x before enumeration; no file mode grant occurs before link inventory.
Only owner read/traverse bits change; POSIX ACL_USER_OBJ is mapped back to original
mode, mask/group/other/named entries are preserved and compared on finish. Original
ctime cannot be restored and is recorded as changed, never claimed byte-identical.

**Residual ACL-intent gap:** for a mode0000 inode, the current implementation
records original mode/identity before grant, then obtains ACL bytes through the
newly readable verified FD and reconstructs ACL_USER_OBJ from original mode. If
it fails in that gap, `acl_captured=false` keeps recovery unresolved. This is
fail-closed but does not satisfy the stronger requirement to persist the complete
original ACL BEFORE granting access. A retained-FD ACL-query primitive (for example
an explicitly supported getxattrat/AT_EMPTY_PATH kernel contract) must be validated
and approved, or this unreadable-ACL case remains blocked. No pathname/procfd
fallback or assumed no-ACL mode is used.

Linux O_PATH + fchmodat2(AT_EMPTY_PATH) is required for inode-bound mode changes,
with no `/proc/self/fd`, pathname chmod or root escalation fallback. Kernel/filesystem
support and ACL behavior require real unprivileged validation. read_dir/open_file/
original_metadata are limited to inventoried entries. finish restores children
before parents, verifies modes/ACLs and records completion; Drop/error leaves an
unresolved fenced journal, not best-effort pathname cleanup. Returned readable FDs
must be closed by the caller before finish/guard release; raw File escape makes
this a caller lifetime obligation, not a core-observed FD-count proof.

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
Owner-read retains one FD per object; deployment must admit that FD budget.

Existing43 tests remain (API-migrated); total73 tests are authored. C/D add exact unions, mixed endpoint,
recovery CAS/gaps, >256 explicit archives, runtime descriptor/activation controls,
0000/internal-hardlink restoration and exact removal controls. Every fixture is
AUTHORED_NOTRUN. Unprivileged0000 test explicitly refuses to masquerade under uid0.
A real process negative fixture is authored but NOT RUN. Positive cross-exec `_exit`,
parent crash/descendant retention, beacon failure propagation, ACL filesystem matrix
and public B1–B3/normal cold-warm-rebuild-retain/prune remain integration gates.
No partial source claim replaces the required full normal-mode validation.
Executed source checks: rustfmt and rustfmt --check exit0, git diff --check exit0,
Python static ownership/API/old-test-name checks exit0. These did not compile any
Rust or execute a fixture. The complete C/D contract is still BLOCKED by the
beacon error-propagation and pre-grant ACL-intent seams above; the code is delivered
incrementally for fan-in and continuation, not a complete normal-mode approval.

Coordinator owns CLI/admin/install wiring and caller operation reuse; no caller,
Cargo/dependency, compat_generation, auth/wire or root-document edits here.
G1/G2/G3 and A4 untouched. Actual migration still requires OS-peer provenance,
old writer/credential retirement, sanitization/profile, authorized CURRENT handoff
and8×2 guest campaigns. Source implementation does not authorize bootstrap on this
host, release, Registry or remote Git actions.
