# Worker lifecycle compatibility admission

Source-only Stream A, incremental on `0608e54e220999b61a94bc617f65f4aaab2f803c`.
This boundary refuses unsafe NORMAL operations; it does not enable migration,
authorize an owner, retire a writer or boot an old root. No marker, name, hash,
path or legacy token issues TransitionAuthority, BootAuthority or OfflineGrant.
The private authority constructors still have no production issuer. The offline
Store/tree/fs, G1/G2/G3 and A4 retained transition-proof lifetime are unchanged.

## Fixed composite contract

```rust
pub(crate) fn with_inputs(admission: Lease, paths: &[&Path]) -> io::Result<Lease>;
```

An empty slice returns the original Arc unchanged. Otherwise, the returned lease
owns the original admission and every input admission. Every input is checked
before success; failure drops the partial batch and cannot perform downstream
work. Clones keep the complete object graph alive until the last clone drops.
`revalidate()` traverses every retained lease, pinned identity and discovery
check, not just the last input or the output's owner. Existing worker/local/
transition/revalidate/WriterLease call shapes are unchanged.

The caller must admit its ORIGINAL host paths before canonicalization, claims,
state allocation, cache/download, firmware, copies, mounts or spawning. Relative
inputs are anchored once to the current directory; `..` is refused, including
in absent tails. Existing directories are traversed with descriptor-relative
O_DIRECTORY/O_NOFOLLOW opens; a final regular file is identified without opening
its content. Symlinks, special files, inaccessible components and replaced pinned
inodes refuse. Revalidation also walks newly created tails and rediscovers
custody; it is not a substitute for a retained fence. Callers must not change
cwd or reinterpret the admitted spelling while using it.

`reject_retained_path` is only a read-only rejection helper. It retains no fence
and must not substitute for `with_inputs` when a path remains in use.

## A1: a domain cannot move as hierarchy appears

For a worker base B, the fence is EXACTLY `B.parent()`. That directory must already
exist in full, must not be `/`, and must remain the same inode. Admission refuses
before normal effects if it does not exist. It does not lock the deepest existing
prefix and does not create/provision directories. The administrator/caller must
provision the owner directory separately BEFORE asking to use it; no permission
to make that change is implied by refusal.

Normal operations take SH, transition takes nonblocking EX on that same domain.
Once admitted, B, its vm_id directories, scratch, root and image tails may be
created without changing the fence. A transition still conflicts until the last
lease clone releases. The private transition API still requires real authority;
the tests exercise only its internal exclusion primitive, not fabricated proofs.

Only the exact domain is flocked. Other ancestor descriptors pin identities but
carry NO flock. There is no root-wide EX (or SH), no lockfile, and no flock on B,
whose exclusive build-dir claims remain independent. Distinct owner domains can
operate while another domain has EX; owners sharing a domain conservatively
serialize transitions. Unsupported directory locking is fail-closed.

This deliberately narrows cold-start compatibility: missing owner parents and
root-level bases refuse rather than accepting an unfenced operation. A missing B
with an existing owner is still normal and causes no admission writes.

## A2: original VM storage, inputs and future enrollment

Custody for `<parent>/<base-name>/<vm_id>` is the sibling namespace
`<parent>/.<base-name>-compat/<vm_id>`. Every id-shaped ancestor of an input is a
CANDIDATE address, including absent directories. Each candidate consults its own
sibling registry, so normal A cannot use original storage belonging to enrolled B,
even across different bases and with an absent root/image. All candidate domains
are retained before admission succeeds, blocking future participating enrollment
of those VM addresses while the source/output/image remains in use. An input
that names an entire base also checks that base's derived sibling registry.

The original admission and the complete input batch supply known bases before
locks are acquired; input order cannot turn another input's base into a claim
lock. A whole known-base/ancestor input checks all its enrollment scopes. An input
under a known base but outside a VM uses that base's domain, not its claim flock.
Ambiguous nested layouts where a selected domain equals a known claim base are
refused instead of risking a blocking claim later. Generic paths with no VM or
known-base relation use their immediate parent as domain; it must already exist.
Missing arbitrary intermediate parents therefore refuse, but a new plain image
in an existing directory and absent VM hierarchies under an existing owner work.

Any addressed per-VM custody entry refuses, including empty/corrupt/symlinked
entries and valid selectors. Existing offline validators remain diagnostic, never
a grant. Derived registry paths must be directories, private0700 and owned by the
effective user; unknown registry entry names refuse. Keyless preparation refuses
any registry, even empty. A normal keyed A remains usable when only B is enrolled
and none of A's admitted inputs touch B or a whole shared base.

## A3: application data is not a migration marker

`active.json`, `receipt.json` and `backup.json` are NOT global markers. A generic
`compat` directory or compat-mode/compat-generation file in a project or unrelated
ancestor is normal application data. Discovery reserves only the derived
`.<base>-compat` namespace and contextual legacy VM markers.

For worker(base,key), the VM address is explicit. Local/input discovery consults
all candidate sibling registries, but applies legacy `<vm>/compat`, compat-mode
and compat-generation checks only with the canonical `<id>/root`, `<id>/upper.ext4`
or `<id>/build-*` runtime layout, or a derived registry. A hex-named project alone
is not a legacy VM. Legacy markers are checked at the VM directory, not in its
root/application children. Names/layout are conservative discovery, NOT proof of
ownership or authorization. Generic records outside a bound custody namespace
cannot identify retained data by themselves.

## Lifetime and integration boundary

WriterLease is unchanged: explicit successful reap releases; cancellation or an
unknown wait conservatively leaks retained leases for the adapter process lifetime.
That can prevent later transitions until a controlled shutdown. Drop, a selector,
a successful wait of one child or an error return proves no global retirement.

Stream B owns caller coverage and pre-effect ordering: WorkerVms.prepare/start/
build/destroy/launch; local, bundle, binary and image builders; and __vm-boot's
root/lower/upper, mount, source_sync and source_readonly inputs. This commit does
NOT establish those callers use the composite. Retention through blocking tasks,
VmCommand and successful wait must be reviewed in the integrated delta. Existing
stop/status/logs, auth, build claims, prune and wire behavior are not edited here.

Cooperative advisory exclusion does not inventory arbitrary directory descendants,
external build claims, bind mounts, hardlink aliases, alternate base spellings or
unregistered host exports. The coordinator must bind/inventory actual original,
source and independent backup storage before authorizing enrollment, and ensure
shared directories cannot expose other enrolled subtrees. A plain backup.json
outside a bound namespace is intentionally not an ownership registry. Nonparticipant
renames/writes, old binaries and process crash still require real all-writer
quiescence, alias inventory and retirement; revalidation cannot make them safe.

Remaining gates: independent core/Stream B seam review; authorized compilation and
focused tests on supported locking filesystems (including normal cold-start and
relative-path regressions); real local OS-peer issuer/provenance; all-writer and
consumer/credential retirement; secret sanitation; exact historical boot profile;
administrative intent and authorized CURRENT/rebuild handoff; then eight origins
by two guest campaigns. No production issuer, compatibility boot consumer, remote
migration endpoint, OLD-over-NEW reimport, publishing or release is added.

## Evidence level

The core suite is authored only. It covers missing-domain refusal, creation after
admission, distinct owners, claim noninterference in both input orders, sibling
B enrollment, absent inputs, whole-base input, symlink/corrupt/unknown namespace,
application marker positives, clone/last-drop/empty batch, partial failure cleanup,
all-component revalidation and explicit WriterLease reap. Own-temp fixtures do
not spawn processes, mount or invoke the offline Store/syncfs, but they have NOT
been executed or typechecked. Source formatting and static/diff checks are the
only permitted checks in this increment. Historical34 offline tests and the
base's28 lifecycle tests are historical authored counts, not runtime evidence.
Nothing here certifies a full migration or a guest runtime PASS.
