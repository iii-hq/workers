# Private offline whole-generation storage (Linux)

This module implements real offline filesystem copying and durable generation
selection. It is **not a migration endpoint or boot feature**. No production
constructor issues `OfflineGrant`; only tests do. No `worker::*` handler imports
or calls this module, and no feature, S-code, contract version or release changes.
A hash, path, JSON record, old bearer token or a caller boolean cannot create a
production grant. `#[allow(dead_code)]` on the private module is intentional while
that reviewed coordinator seam is absent.

## Exact internal seam

`OfflineGrant` is private, not Deserialize. It retains binding64hex,
Environment, a no-follow source Dir, pinned Inventory, and an exclusive source
lease. A future local OS-peer coordinator must create it ONLY after proving exact
old artifact/lower/boot continuity, clean stop, fenced old writers and credential
retirement at the consumer. Its lifetime must cover all operations and any later
VM writer. The adapter currently has no such issuer: default external path stays
unchanged/unavailable, not a JSON-authorized capture.

`Store::open(&grant, custody:Dir, backups:Dir, protected:&[&Dir])` validates
existing private0700 roots, nonempty protected inventory, ancestor/dev/inode
separation and source fingerprint, then holds nonblocking exclusive directory
flocks. Custody is outside removable VM/build roots; backups are outside custody,
source and EVERY supplied managed/share root. Retained descriptors and revalidation
reject substituted ancestors. The coordinator must resolve mount aliases and
supply the COMPLETE root inventory. Unenumerated bind aliases/old uncooperative
writers are not made safe by advisory locks.

Operations (private, no public wire exposure):
- `capture(operation, expected=0, faults)` snapshots the authenticated source once;
- `selected()` rereads/validates active selector and immutable lower, returns a
  held offline data view tied to the store lease, never boots a VM;
- `rebuild(operation, expected, &same_environment, faults)` snapshots CURRENT
  active bytes, including new writes; no application hook is run in this step;
- `recover(operation, expected, faults)` publishes only a complete verified
  generation or acknowledges an already-selected operation after directory fsync;
- `rollback_copy(operation, &fresh_isolated_destination, faults)` exports its
  retained backup without changing selection, originals or newer writes.

Operation ids are1..64 ASCII alnum/hyphen. Hashes are lowercase64hex. Format1 is
LOCAL STORAGE ONLY, not sandbox wire-contract1.5. All JSON roots are object-only,
unknown/duplicate fields rejected; nested Environment is map-only too. Records
are<=64KiB, private0600 regular single-link files. No secret/token field exists.

Environment exact fields: source_version (`0.3.0|0.3.1`), layout
(`overlay_image|legacy_clone`), provision (`path|bundle`), adapter_sha256,
engine_sha256, kernel_sha256, init_sha256, profile_sha256, app_sha256,
sdk_lock_sha256, provisioning_sha256, lower_sha256 (overlay required/clone null).
These are integrity bindings, not authentication. Profile digest pins the exact
historical profile; this module does not assert the new kernel supports it.

Record exact fields: version, binding, operation, generation, parent, parent_operation,
environment, backup, backup_digest, snapshot_digest. generation=parent+1 checked
without overflow; parent_operation must match the current selector for noninitial
publication; backup id=operation. Missing selector with multiple retained generations
is ambiguous and refused, never used to roll back implicitly. Selector exact fields: version, binding,
generation, operation, record_sha256. Backup metadata: version, binding,
operation, environment, snapshot_digest.

## Input generation and fidelity

The coordinator must supply an owner-private offline capture envelope with ONLY:
- overlay: `upper.ext4` (whole original disk bytes) + `lower.erofs` (exact bound
  immutable bytes); no extraction or raw-overlay metadata reinterpretation;
- legacy: `root/` (whole clone, not application-directory heuristics);
- bundle: additionally `bundle/` with the separately owned external source/data.

It is NOT safe to produce that envelope by following a guest JSON path or
reconstructing a lower by tag/mtime. Current legacy install layouts are not
implicitly rearranged by this primitive; the real coordinator must bind/make
available the actual owned source handles without losing metadata/provenance.

Pre-inventory opens held dirfds with NOFOLLOW/O_NOATIME. It verifies entry identity,
ctime and links, rejects unsupported types before staging, hashes all regular file
bytes, and keeps a canonical sorted metadata manifest in memory. Copies use EXCL
creation, sparse SEEK_DATA/HOLE extents, file/directory fsync, numeric uid/gid,
modes, nanosecond atime/mtime, complete xattrs (POSIX ACL/capabilities included),
hardlink groups and symlink target bytes. Every copied entry is reread and compared
to the source manifest; no dropped metadata success. Source is re-inventoried
before/after copy; original entries are never renamed, deleted or overwritten.

Precisely unsupported (refused, not migrated/PASS):
- non-Linux builds; unknown versions; missing components/lower digest mismatch;
- special files, devices, FIFOs or live sockets in clone/bundle trees;
- hardlinks crossing the capture boundary or hardlinked symlinks;
- nested mount/bind points detected via statx mount id; unavailable mount identity;
- immutable/append/encrypted/project-inherit/other inode policy flags (ordinary
  extents/directory-index implementation bits allowed);
- non-root copy of foreign numeric uid/gid, unavailable O_NOATIME/xattr/seek APIs;
- symlink trees unless source AND destinations are noatime filesystems (readlink
  can otherwise change original atime); no silent timestamp repair on originals;
- more than16,384 entries, depth128/path4096 or xattr list/value over1MiB;
- inability to restore ACL/security xattrs/ownership, or fsync/no-replace primitives.

A device node/unsupported tree is rejected at inventory, before backup/staging
mutation. Destination ACL/xattr permission/storage failures may occur during a
copy; they retain incomplete staging and source, never publish it. ctime/birthtime,
physical inode numbers and physical block locations cannot be preserved as source
identity; ctime is used for change detection, not copied. Holes' logical bytes are
preserved; allocation layout is not asserted identical. No clone/OS conversion or
appdata exclusion occurs. Secrets inside the old root remain in private offline
copies; sanitization/revocation BEFORE boot is still required elsewhere.

## Durable protocol and recovery

Preflight checks profiles/components/identity/support/collisions and protected
roots. Same-filesystem staging/new-name publish uses Linux RENAME_NOREPLACE; no
check-then-overwrite fallback. Names are fixed-derived, never arbitrary paths:

```
backups/stage-<op>/data + backup.json
backups/<op>/data + backup.json                 # completed first, independent
custody/stage-<op>/data + receipt.json
custody/generation-<op>/data + receipt.json     # complete, not yet selected
custody/selector-<op>                          # fsynced new selector record
custody/active.json                           # atomic regular-file selector
```

Copy+verify+fsync content and directories, then backup rename+parent fsync. Make
and verify candidate, write/sync receipt, publish generation with no-replace,
fsync parent. Under the held custody lock, compare current generation, validate
backup and candidate, sync selector, atomically publish active and fsync parent.
For replacement, rename overwrites ONLY the validated selector record (old
generation+backup remain); first activation uses no-replace. Any post-selector
fsync failure returns Uncertain, not success or automatic rollback. Recovery
rereads active and syncs; operation replay never reimports OLD over NEW. Incomplete
staging is retained and not adopted/bootable. Full unselected generation recovery
requires exact backup/candidate digests. There is no cleanup/prune API.

After commit, snapshot_digest is historical capture integrity, NOT an assertion
that live data must stay frozen. selected validates the immutable lower and current
supported structure, not OLD content equality. Restart/rebuild selection does not
read or copy the old source again. Rebuild must retain exact environment; changed
OS/lower/app/profile is a separate explicit migration. Original source can change
after activation without overwriting NEW. Explicit rollback copies a retained
snapshot into a fresh isolated directory and retains current NEW for reconciliation.

## Not yet implemented at the product boundary

Real OS-peer grant issuance/cross-process all-mutator fencing, authentic published
artifact and old-lower/profile observation, old authority retirement, secret
sanitation, selector admission before normal build claims/next_instance, actual
VM boot using these retained handles/profiles, same-app reprovision hooks, negotiated
destroy intent and real historical smoke harness remain integration work. Existing
start/archive behavior is NOT fixed by merely adding this private storage module.
No claim of a working general upgrade or support for all legacy trees is made.
The eight historical source/layout/provision cells remain NOTRUN; two campaigns
require16 later actual guest cycles, not synthetic disk marker tests.

Tests in tests.rs exercise the actual filesystem implementation with small byte
fixtures, fault boundaries and OLD/NEW semantics. They are not ext4/EROFS validity
or guest-readback proofs. Compile/runtime require independent review, current
admission and explicit shared-slot grant. Existing contract1.4/auth/hooks/destroy
retain/prune behavior is untouched.
