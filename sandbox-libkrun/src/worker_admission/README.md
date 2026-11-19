# Admission v2 core — source candidate, NOT runtime acceptance

Base: `4dc1417456704f6fd2b42220f49c245710564876` (tree
`78797f20a4aa98ad000301cb5e621a57bb47d368`). This replaces the old moving
ancestor-directory flock scheme. It implements the approved v2 source seam and
amendments A/B, but **the complete delivery remains blocked by the explicit
integration/protocol gaps below**. No normal-mode PASS, typecheck, migration,
bootstrap, socket, process, VM or power-loss test was executed.

## Implemented source

- `AdmissionContext::{system,worker,local,reject_retained_path,import_child}`;
  production free functions retain their signatures. Context is carried by every
  lease. `testing::{provision_context,open_context}` exists only under cfg(test),
  uses an explicit private own-temp root and does not modify env/HOME/cwd/globals.
- `bootstrap_for_uid` requires effective uid0 and creates only the fixed
  `/var/lib/iii/worker-admission/v1/<euid>` layout via no-follow retained directory
  descriptors, EXCL creation, ownership checks and fsync. Existing incompatible or
  partial material refuses. `system` and `check_installation` do not bootstrap.
- Durable bounded catalog, registered bases, raw-byte absolute path encoding,
  ancestor/descendant intervals in both directions. Unknown schema/versions,
  namespace entries, symlinks, wrong modes/owners/links and replaced lock inodes
  refuse. SHA envelopes detect torn/corrupt records; SHA is NOT authority.
- Short nonblocking catalog flock; one exclusive operation sentinel flock plus
  Linux OFD byte lock. Data/claim directories are never flocked. `with_paths`
  extends the same union atomically, preserving clones and prior coverage on
  failure; `with_inputs` means Strict. Missing tails do not choose lock domains.
- Mutable intervals reserve writes against other operations. Strict intervals
  share with Strict; transition conflicts with either direction of overlap.
  Registered-base legacy discovery includes a whole `<base>/<id>` input, without
  treating a generic hexadecimal application or generic receipt/compat name as
  a VM. Custody entries still refuse, even valid-looking selectors.
- `UseGuard` registers a read hold and validates its own exact snapshot;
  overlapping mutations refuse, disjoint reads remain possible.
- Nonclone `MutationGuard`: create directories, issue EXCL sibling file/tree
  stages, publish by retained-parent NOREPLACE rename, preserve the prior version,
  adopt by NOREPLACE, journal effects/epochs, and issue private EffectReceipt.
  Commit updates only identities observed by those owned effects, never generic
  current-inode refresh. Panic/drop after artifact effects retains uncertainty.
- `DiagnosticLock` exposes borrowed AsFd, not a mutable File. It requires an exact
  MutableFile sidecar and related declared artifact. Existing0644-compatible
  nlink1 regular lock inodes are opened without truncation, created EXCL otherwise,
  never staged/renamed/unlinked. Acquisition writes nothing; PID/clear require
  this handle's flock and terminal validation. No catalog mutex survives a wait.
  Handles retain the operation; commit refuses outstanding handles; Drop writes
  nothing. Ordinary refused acquisition before artifact effects can abort cleanly.
- `discard_file` consumes only this mutation/context/operation's unpublished
  regular stage. Exact MutableFile semantic slot required. Durable DiscardIntent
  binds stage ID, parent identity and file identity before unlinkat of ONLY the
  generated staging name; fsync precedes durable Discarded. Target is untouched.
  Disposal appears separately in EffectReceipt. Failure poisons and preserves
  journal, never retries by filename or promotes a pending output. Drop does not
  resolve an abandoned stage. Caller MUST join writers and close spool clones.
- Child export duplicates the exact sentinel OFD CLOEXEC. Import validates record,
  namespace and inode, observes an OFD conflict through an independent open and
  no conflict through the inherited description, and increments the same operation
  holder count. It never LOCK_UNs a shared description, never treats PID/string as
  migration proof, and rejects wrong-context/independently opened descriptors.
- Endpoint role, private parent outside payload intervals, durable socket receipt,
  verified stale ownership, bind/close identity comparison and conservative unknown
  lifetime retention. Unknown/live/foreign endpoints are not probed or deleted.

## Safety and storage bounds

Per namespace: 256 operation records,256 registered bases,256 endpoint receipts.
Per operation:256 intervals/holds/journals,8192 pins;128 components/path,4096 raw
path bytes;512 journal changes,256 artifacts/disposals. Serialized record cap4MiB,
payload cap2MiB; current and previous snapshots have fixed checked inodes. Tree
sync is capped4096 entries/depth64. Catalog memory/work is bounded by these caps;
exceeding them is an error, never omission. Metadata scans do not inventory guest
contents. Same-UID/admin is the trust boundary, not hostile admin containment.

Before current-catalog overwrite, the prior complete snapshot is fsynced to
`catalog.previous`; corrupt current never silently falls back. Unknown sentinel
or torn namespace remains fail-closed. Closed records WITHOUT journals can be
reconciled only after obtaining the verified sentinel. Journal-bearing records
and preserved prior copies remain; currently the256-operation bound can therefore
require administrative recovery/archival (not implemented). No automatic prune.
A lock/filesystem/platform without Linux OFD/NOREPLACE support refuses; this is
not counted as a normal-mode or migration PASS.

Lexical scopes do not resolve bind aliases, alternate mounts, guest shares hidden
from declarations, old binaries or uncooperative same-UID writers. Coordinator
must inventory aliases and provenance and retire nonparticipants. Revalidate plus
rename/unlink is not hostile-writer atomicity; private retained parents and the
cooperative protocol are prerequisites. An inode pin does not hash input bytes.
The offline Store/tree/fs, G1–G3, A4, auth/wire and actual consumers are unchanged.

## C1–C6 matrix (AUTHORED_NOTRUN)

| Finding | Source behavior | Remaining integration condition |
| --- | --- | --- |
| C1 | Nested/whole-base intervals and new registration compare both containment directions; post-creation domain cannot move | Cross-process test, registry lifecycle admission |
| C2 | Chained calls preserve one union and hold no data/claim flock | Same-operation propagation through all helpers; S505 tests |
| C3 | Cold state/runtime and output/cache declared mutable trees can be created under a stable namespace interval; exact descendants use receipts | Root must cover all missing intermediate directories; callers must declare outputs, not Strict aliases |
| C4 | File/tree replacement, adoption and epoch updates are core-owned; old copies retained, external substitution terminal | EROFS spool-plan amendment below; recovery/archival; actual writer lifetime |
| C5 | Catalog bases identify whole VM legacy, including empty tail; generic names remain normal | Trusted registration before effects for every base/claim scope |
| C6 | Dedicated endpoint operations restart only with matching receipt and released sentinel | Mixed operation crash recovery, listener close ownership and parent preparation below |

Tests in `tests.rs` exercise the real catalog/context and syscalls, not bool
migration authorities. They cover cold/warm creation, nested/chained intervals,
claims, clone/child import, corruption/namespace substitution, strict pins,
owned/external publication, diagnostics, stale/live/unrecorded endpoints and
spool disposal. Five disposal boundary faults are authored. Socket fixtures are
SOURCE ONLY; even those were not run. Existing outside-module tests still need
context migration by their owner; no implicit production fallback is provided.
Independent-process actual descendant lifetime, filesystem unsupported, uid-owner
negative fixtures, power-loss and B1–B3 public-build integration remain unexecuted
and not fully authored here. A test crash model is explicitly cfg(test), not a
production retirement issuer.

## Blocking gaps and minimal contract decisions

1. **EROFS temporary spool footprint:** `Erofs { output }` authorizes exactly
   output plus its declared diagnostic sidecar, NOT a separate sibling spool slot.
   `ImagePair { directory }` supports explicitly declared spool + image with
   discard-before-publish, but may collide with Strict inputs in that directory.
   Proposed minimum amendment (NOT implemented):
   `MutationPlan::Erofs { output: PathBuf, spool: PathBuf }`, with exact distinct
   MutableFile declarations, one guard holding both footprints, and discard only
   for that slot. Do not accept arbitrary `.lock`/spool siblings.
2. **Same-operation helpers:** two independent `local(state)`/`worker(base,key)`
   calls overlapping a mutable tree correctly return Busy. Helpers must reuse
   the caller Lease, not merely the same AdmissionContext. Child import provides
   this across exec; caller flow still requires integration by Stream B.
3. **Endpoint mixed-operation restart:** a dead receipt from an operation also
   protecting VM/data/writer journals remains refused. Current stale restart only
   proves a dedicated endpoint-only operation. Proposed minimum: endpoint-specific
   sentinel owned by EndpointLifetime, retained/verified separately from operation
   writer uncertainty, with exact retirement/recovery rules. Do not clear an
   entire mixed operation to clean its socket. `into_parts` also relies on the
   caller closing EVERY listener clone before `close`; core cannot prove this
   from a raw UnixListener or PID. Parent must confirm that ownership seam or
   amend it to an owned listener wrapper. New endpoint parent must already be
   private/outside payload; prepare it with a separately admitted mutation.
4. **Recovery/archival:** no production API resumes poisoned/discard-intent or
   orphaned mixed-operation journals. Evidence stays fenced. Need a private
   explicitly authorized recovery contract, verifying IDs/parent/file/current
   absence and durability without reimporting OLD or deleting replacements.
   A corrupt current catalog cannot be replaced by the previous snapshot without
   reconciling later sentinels/intents. Completed journals also need bounded
   archival before catalog exhaustion; no ad hoc cleanup was invented.

These gaps mean the delivery is PARTIAL/BLOCKED, not completed C1–C6 or upgrade.
They are not a return to the superseded OwnerScope proposal: real catalog and
operations are implemented here and preserved for integration.

## Installation checklist — documentation only

No command was run and no production directory was created. Coordinator owns the
CLI spelling (none is invented here). After separate operator authorization:

1. Confirm Linux OFD flock and renameat2/NOREPLACE on the chosen filesystem,
   available metadata capacity, uid and administrative ownership of `/var/lib`.
2. The coordinator's explicit administrative CLI invokes
   `bootstrap_for_uid(<numeric-euid>)`; this is never a worker endpoint or normal
   prepare/start/build side effect. Do not chmod/chown incompatible existing data.
3. The target user invokes the coordinator's read-only `check_installation` route.
4. Keep the namespace outside payload/export/guest paths. Retire old processes;
   do not infer quiescence from bootstrap success. Provisioning alone does not
   authorize migration, secrets, credentials or a boot profile.

## Validation status

Only existing rustfmt and static/diff checks are permitted in this source round.
No Cargo/rustc, Rust test, socket/process/VM fixture, mount/syncfs, networking,
download/install, cleanup, publication or release. Source receipt records exact
check exit codes, commit/tree and authored counts. Full admission, shared-target
build/test matrix, combined review and authenticated historical8×2 campaign remain
coordinator gates. One consolidated Workers PR; releases remain operator-owned.
