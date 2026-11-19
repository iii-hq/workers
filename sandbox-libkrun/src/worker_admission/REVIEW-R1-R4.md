# R1–R4 correction protocols — source candidate

Base35e5d0b980109b25264bc66247f82f885f8b5233/treefececc7eb55db5f725ec7ad8452dea1d7f54d74c.
Branch `fix/migration-review-core-r1-r4`. The partial9730cef clone branch is untouched;
no wholesale merge, custody copier export or read-only0000 authority claim occurs.
R5 and existing runtime_tests.rs belong to B and were not edited.

## R1/R2: one bounded ordinary-log record, stable file descriptions

```rust
struct LogSpec<'a> {
    current: &'a Path,
    previous: Option<&'a Path>,
    cap_bytes: u64, //1..64MiB; previous required for continued rotation
}
impl Lease { fn open_log(&self, spec: LogSpec<'_>) -> io::Result<LogWriter>; }
// Clone only shares the SAME mutex, cursor, original Lease and exact file FDs.
impl LogWriter {
    fn append(&self, record: &[u8]) -> io::Result<LogCursor>;
    fn read(&self, offset: u64, max_bytes: usize) -> io::Result<LogRead>;
    fn cursor(&self) -> io::Result<LogCursor>;
    fn reset_instance(&self) -> io::Result<LogCursor>;
    fn finish(self) -> io::Result<LogCursor>;
}
struct LogCursor { base:u64, len:u64, previous_base:u64, previous_len:u64 }
struct LogRead { bytes:Vec<u8>, next:u64, end:u64, oldest:u64 }
```

Declare exact MutableFile current/previous paths and create them ONCE through the
existing owned mutation before open_log. Parents are private0700; files0600/owned,
nlink1, no-follow, expected inode. open_log never creates/truncates a payload. It
reuses the SAME ledger on later clean reopen, proving identity, sizes and content.
New-instance retention reset is EXPLICIT via reset_instance, never implicit on open.
Both reset and append journal a bounded pending outcome before any effect.

Each append is write_all_at on the retained FD. Once accumulated length exceeds
cap, copy current into the already-owned rotated FD, sync, then truncate current;
this matches existing rotate-after-whole-record policy. Inodes never change.
Catalog entries/journals do NOT grow per line or rotation: one record (max16 logs)
contains current cursor, sequence, previous cursor, cumulative chain digest and
one Pending outcome. Current SHA state is incremental in the writer; append is
O(record bytes), rotation O(cap), finish/reopen/recovery hashes retained windows.
No per-line whole-log stage, .iii-prior or .iii-stage payload file is created.

The pending slot captures expected final hashes/cursor before writes. Only after
fsync and exact identity/length validation does a completed outcome update cursors.
The first effect or persistence failure is sticky; another clone cannot retry from
old offsets. A pending crash is not blindly replayed. Explicit administrative
recovery with free operation/writer sentinels can reconcile exact completed final
bytes/hashes/lengths, never append again or truncate a replacement. Partial rotation
remains unresolved. Recovery does not infer child termination from a log state.

All writable FDs stay private in LogWriter; no AsFd/File/try_clone/path writer escape.
Concurrent clones serialize on its mutex. finish requires the last clone, hashes
final retained bytes, syncs and removes only the two exact log holds after durable
Finished. Drop without finish marks uncertainty and retains obligations. Runtime
retirement does not retire this parent writer. F receipt Remove and H1 continuation
remain blocked until every log is Finished and no pending work exists. The same
metadata row is archived with the operation; no history is silently deleted.
Ordinary bounded log retention discards only declared log windows according to cap;
real .iii-prior, backup/generation/compat evidence remains forbidden as a log path
and protected by existing Remove.

### Caller routing (required, NOT edited here)

- WorkerVms spawn initializes exact console.log and console.log.1 only once, opens
  LogWriter before launch, stores it in Console/pumps. No new stage on append.
  Keep caller JSONL formatting; call append(bytes) and use returned cursor only
  after success. Route reads through LogWriter::read (or only after finished), not
  independent readers racing the in-place rotation. New instance may explicitly
  reset retention. Existing wire cursor policy remains caller-owned.
- sandbox initialize_stderr creates stderr + declared stderr.1 once, then the pump
  owns a cloned LogWriter. append_stderr calls append, not MutationGuard. The pump
  returns its clone; join all pumps and call finish before receipt.begin_remove.
  Do not silently discard stderr errors or mark registry stopped on Failed.
- Clone/read an existing legacy file is not authority to relabel it as ordinary
  log. Only exact declared original operation paths can issue the capability.
- The64MiB hard cap and1MiB record limit are resource limits. Select existing
  console cap/explicit sandbox retention policy; out-of-profile errors are not a
  claimed successful stop. No upper/guest-data path is in the log footprint.

## R3/R4: exact inventory -> core producer -> opaque seal

```rust
struct PayloadMetadata {
    mode:u32, uid:u32, gid:u32,
    atime_sec:i64, atime_nsec:i64, mtime_sec:i64, mtime_nsec:i64,
    xattrs:BTreeMap<Vec<u8>,Vec<u8>>,
}
enum PayloadKind {
    Directory, File { size:u64 }, Symlink { target:Vec<u8> },
    Hardlink { source:PathBuf }, // internal planned primary file
}
struct PayloadEntry { relative:PathBuf, metadata:PayloadMetadata, kind:PayloadKind }
impl PayloadInventory { fn new(entries:Vec<PayloadEntry>) -> io::Result<Self>; }
impl MutationGuard {
    fn produce_payload(self, target:&Path, inventory:PayloadInventory)
        -> io::Result<PayloadBuilder>;
}
impl PayloadBuilder {
    fn write_file(&mut self, relative:&Path, reader:&mut dyn Read) -> io::Result<()>;
    fn seal(self) -> io::Result<PayloadSeal>;
}
impl PayloadSeal {
    fn publish(self) -> io::Result<()>; // consumes original guard and commits
    fn publish_into_guard(self) -> io::Result<MutationGuard>; // cache digest follows
}
impl Lease { fn capture_payload(&self, root:&Path) -> io::Result<ReadablePayload>; }
impl ReadablePayload {
    fn produce(self, guard:MutationGuard, target:&Path) -> io::Result<PayloadSeal>;
}
```

No arbitrary path, bool, PID, caller completion receipt or external stage can be
sealed. Inventory is data, not authority: the existing exact Reclone/cache-entry
guard binds operation, epoch, target and retained private parent. The core alone
creates files/dirs/links via retained no-follow dirfds; it supplies no writable FD
or stage pathname to consumers. `write_file` consumes exactly declared bytes from
a reader and fails on short/extra/second write. It can run under a single actual
owned blocking task; caller joins that task. Cancellation retains the moved guard
until real completion; no future-to-bool proof. External descendants can retain
input readers but cannot get destination writer FDs.

Seal verifies complete production, closes ALL core RW file descriptors, then
applies original declared metadata via retained descriptors. Private coordination
stays0700/0600; payload1777/0777/0755 is legitimate inside its private parent and is
not sanitized. Directory metadata closes child-before-parent. Internal hardlinks
share the one written inode and exact metadata; external link count mismatches
refuse. Snapshot identity includes gid/mtime/ctime, directory member inode sets,
bytes hash, exact link groups and requested ACL/xattr/timestamps. New external
mode/inode/member changes refuse, not refresh. Same-object Identity semantics are
not weakened globally.

Source capture for readable clone opens O_NOATIME descriptors under a Strict
UseGuard, inventories metadata/xattrs/links/mounts before destination effects, then
copies bytes through the SAME producer. Original source pins/ctime/read scope
remain until publication; the caller never invokes cp -a. Sparse clone data/hole
maps are copied/verified with SEEK_DATA/SEEK_HOLE; no silent dense fallback.
Unreadable0000 is not automatically granted permissions here. The preserved partial
clone work and user-approved shared0000 refusal are separate gates, not smuggled
into this review correction.

The seal records exact finalized root identity and inventory/byte evidence in the
same original journal, then invokes existing preserve-prior + NOREPLACE publication.
The authorized root0700->0755 change is only from this core writer transition;
it does not call generic reobserve or let an external producer change stage attrs.
Existing `publish_tree(StagedTree)` remains restrictive for legacy callers. Routing
them to this new producer is required to fix production, not merely increasing
sync_tree4096 or ignoring its failures.

### Resource profile, not an unbounded limit increase

The final inventory is admitted BEFORE stage creation: exact count/bytes/xattr
memory, parent/mount/ownership, statvfs inode/disk capacity with10% local margin and
actual FD reservation for3*entries+64. Platform maximum262144 entries,64GiB data,
64MiB inventory metadata,128depth,4096path bytes; per-file xattr bound1MiB, clone
extent bound65536. This is a separate producer profile sized to admit5001 and much
larger real rootfs trees; it does not alter MAX_PATHS/MAX_JOURNALS or sync_tree's
legacy bound. The catalog stores one inventory digest/size receipt, not an entry
per payload member. Directory/group indices avoid quadratic inventory traversal.
Resource pressure is an error before stage effects where known; actual I/O failures
retain the guard/journal. No coordinator reservation is reduced or disk cleaned.

Native64 Linux xattrat (verified E UAPI463–466) preserves exact supplied metadata;
errors such as unsupported ACL/ownership/security-xattrs are explicit, never dropped.
Readable symlink source requires noatime to preserve source atime. Parent fsync and
retained file/dir fsync are used; no new syncfs/mount/procfs/root fallback. Thus the
full0000/symlink-crash-fidelity matrix remains a separately documented platform gate,
not an assertion that the small metadata examples prove every filesystem.

### Exact caller examples

```rust
// OCI: pre-scan local tar/layers under real owned input guards and resolve
// whiteouts/hardlinks into a FINAL inventory (same existing sanitize policy).
let inventory = PayloadInventory::new(final_entries)?;
let guard = prep.begin_entry(&selected_root)?;
let mut producer = guard.produce_payload(&selected_root, inventory)?;
for final_file in selected_tar_members {
    producer.write_file(&final_file.relative, &mut final_file.reader)?;
}
let mut guard = producer.seal()?.publish_into_guard()?;
publish_digest_in_same_guard(&mut guard, &digest_path)?;
guard.commit()?;

// Readable non-overlay source with0755 root:
let source = admission.capture_payload(&selected_root)?;
let guard = admission.begin(MutationPlan::Reclone { root: destination.clone() })?;
source.produce(guard, &destination)?.publish()?;
```

`rootfs_cache::resolve_with` and OCI `pull_into_stage_checked` must change from
external extraction into a returned stage path to final layer-plan/readers feeding
PayloadBuilder. Parent must route this producer and readable clone before R3/R4
are accepted. No caller source or tar policy was changed here. An actual local tar
reader regression is authored; full multi-layer/whiteout caller coverage belongs
to B/coordinator. Root metadata and hardlink groups originate in the declared
supported inventory, not an untrusted completion marker.

## Tests/evidence

Baseline35e5 contains169 core test entries. Current192 entries preserve all of them
and add23: log_tests12 (11 scenarios + one own cross-exec helper), payload_tests11.
Counting the existing G helper, current totals are190 scenarios +2 helpers.
Compiled0/executed0. All new tests are in log_tests.rs/payload_tests.rs; runtime_tests.rs is byte-identical
to35e5 and remains B-owned. Authored tests include512append bounded cardinality,
350cleanreopen, concurrent clone lifetime, cursor rotation, intent/effect/rotation/
outcome faults, reset retention, recovery without reappend, replacement refusal,
logged own-layout removal preserving evidence rules; payload0755/1777/0777,
5000files, internal hardlinks, real local tar reader, readable source clone,
metadata/seal failures, foreign/unplanned paths and member replacement. No test
was compiled or executed. Source checks: rustfmt and rustfmt --check on12 edited
Rust files exit0; git diff --check exit0; bounded Python exact-base/new-branch/
14-owned-path/API/old-test checks exit0. Existing runtime, runtime_tests, lineage,
pidfd, endpoint, storage and prior standalone tests remain byte-identical to35e5.
One no-op regex edit request missed its anchor and wrote nothing; no functional
workload was executed or retried. These checks do not replace typechecking. Historical14PASS/9PASS/build evidence remains unchanged
and does not cover these new flows. No review fragment or new acceptance is claimed.

Next steps: parent checks this immutable interface, routes consumers, admits exact
R1–R5 reproductions and combined matrix with firstFAIL preservation, then the SAME
reviewer completes semantic coverage. No deadline exception, remote push, new PR,
Registry/release or source-only syscall fixture execution occurred.
