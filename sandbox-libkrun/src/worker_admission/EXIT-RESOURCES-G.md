# G: runtime-owned exit resources

Source increment after `71e7d6660b746bf51ed6981f67185c36cba262f0` (tree
`1981c3e4e1fdb29880dc4021b2a8e8422a816d2f`). F lineage is already implemented
and preserved, not a pending dependency. The G contract SHA read was
`54661eaff93cb4ea2e38dc82fe5121c51050cc13482fd64630b03ce94dce206a`.
No production/caller route, process, socket, fixture, Rust build or test ran here.

## Exact new crate-private API

```rust
impl RuntimeWriter {
    pub(crate) fn use_path(&self, path: &Path) -> io::Result<UseGuard>;
    pub(crate) fn endpoint(&self, path: &Path) -> io::Result<EndpointGuard>;
}
impl WriterRetirement {
    pub(crate) fn retire_runtime_endpoints(
        &self, lease: &Lease, paths: &[&Path],
    ) -> io::Result<()>;
}
```

Existing F/D/E signatures remain. Runtime-issued UseGuard, EndpointGuard,
BoundEndpoint and EndpointListener internally retain the actual RuntimeWriter and
its dedicated sentinel. No raw sentinel/FD accessor or caller runtime-id parameter
exists. A surviving read guard, relay listener wrapper or descendant descriptor
prevents the independent-sentinel proof in the real owned waiter. Generic host
Lease::use_path/endpoint still work for the parent and unrelated imports, but an
activated runtime's imported holder must use the RuntimeWriter APIs. A generic
preboot endpoint guard cannot bind unowned resources after that holder activates.

## Persistent ownership and legacy records

New holds record the issuing holder and optional `(runtime id, runtime epoch)`.
Runtime epoch is not the operation's later mutation/endpoint epoch. Resource IDs,
holder membership, exact runtime admission holder/epoch and permitted kind are
validated on catalog reads; all write/owner-read/runtime hold constructors now
record their actual issuing Lease holder too. Read and endpoint resource IDs are
unique. Endpoints persist their exact hold ID and ResourceOwner in BOTH record
and stable sentinel binding before bind effects. Settling an endpoint removes
that hold ID only, not every resource sharing its path.

Optional ownership fields permit parsing old records, but absent ownership is
unresolved legacy evidence, never inferred cleanup authority. Inconsistent partial
owner/epoch/member/hold bindings refuse. Generic dropped-holder release preserves
membership while an owned or legacy-unknown obligation remains. Normal guard Drop
retires only a matching issuing holder/hold; _exit is handled by the real waiter.
No PID, zero holder count, JSON claim, callback or old path grants ownership.

## What actual writer retirement settles

F's real Child wait, pidfd observation, clean activity slot/shared latch, independently
free runtime sentinel and root/parent checks are unchanged prerequisites. The same
finalization transaction removes ONLY:

- its matching runtime hold (original issuer/runtime epoch verified);
- Read holds whose recorded holder + runtime ID + runtime epoch exactly match the
  imported writer being retired.

Parent reads, sibling runtime reads, unrelated imports, legacy-unbound reads,
mutations, diagnostics, OwnerRead permission work and uncertain journals stay.
Imported-holder membership is removed only if no unresolved obligations remain,
or every remaining obligation is an exact runtime-owned endpoint record capable
of independent retirement. That transfer keeps endpoint holds until explicit
closure. A post-commit receipt-issuance failure preserves the committed transfer
graph and poisons the operation, rather than corrupting it by reverting runtime
membership. Writer-retired still has `guest_consistent=false`.

## Parent endpoint settlement: no bind, no sweep

The private WriterRetirement snapshots eligible endpoint identities at issuance.
The same original Lease Arc, op sentinel, runtime state/epoch and activity outcome
must still match; a consumed Remove receipt or newer runtime/endpoint issuance
refuses. Requested paths are absolute, distinct, bounded256 and individually
anchored. Empty set is a validated no-op, not all-endpoints authority.

For every requested record, before ANY effect: compare id/endpoint epoch/runtime
owner/hold ID/parent/socket/sentinel against the private anchor; verify the private
no-follow parent, exact socket type/uid/link/inode and independently acquire its
sentinel EX/OFD lock nonblocking. All requested sentinel FDs stay owned until the
bounded call returns. Parent/sibling writer or read intervals affecting the private
control directory still conflict. No rebind, parent deletion, admin recovery,
broad-root permission or clearing of data uncertainty occurs.

Close state machine is `Bound -> durable CloseIntent -> exact parent-FD unlink ->
parent fsync -> durable Retired`. A missing Bound socket is an error. A matching
CloseIntent may reconcile the exact file or its recorded absence; it never deletes
a replacement. Already-Retired is idempotent only for the exact anchored record,
not inferred from a missing path. All input paths prevalidate before effects; a
later I/O failure may leave an explicitly mixed Retired/CloseIntent/Bound set.
The error is returned; retry can reconcile only those anchored records. Nothing
marks all resources clean. Successful endpoint closure leaves the SAME one-shot
Remove receipt unused for the subsequent F-lineage removal.

## Caller example (B owns routing)

```rust
// Child __vm-boot already imported both real inherited descriptors.
let runtime = import_runtime_writer(&admission, runtime_fd)?;
let lower_use = runtime.use_path(&resolved_lower)?;
let relay = runtime.endpoint(&shell_socket)?.bind()?.into_async()?;
// Relay tasks own relay; lower_use and runtime stay through the actual VMM.
// Normal close is permitted. _exit skips Drop and is NOT simulated as close.

// Parent owns RuntimeChild and the SAME original lifecycle admission.
let completion = child.wait().await;
report_os_status(completion.os_status()); // separate from successful retirement
let receipt = completion.retirement().map_err(RuntimeFailure::to_io_error)?;
finish_host_pumps_and_control_writes().await?; // visible errors, no swallowed failures
receipt.retire_runtime_endpoints(&admission, &[&shell_socket])?;
// For a no-relay launch use &[]; do not guess an endpoint by pathname.
retain_guest_data_with_owned_adopt(&admission)?;
let mut remove = receipt.begin_remove(&admission, &vm_dir, &exact_runtime_targets)?;
for target in &exact_runtime_targets { remove.remove_path(target)?; }
remove.commit()?;
remove_registry_only_after_success();
```

Retention still follows F's same-operation old/new journal lineage. Prune after
consumed ordinary-destroy receipt, legacy/restart proof, and cleanup of retained
endpoint-parent directories remain explicit policy/capability gates. The endpoint
settlement method does not make receipts reusable or authorize broad cleanup.

## Authored validation and limits

All113 previous core test names remain. Current127 test entries comprise126 scenario
tests and one cross-exec helper. The positive cross-exec fixture execs the current
test executable, supplies explicit test namespace + real inherited admission/runtime
FDs, acquires runtime read/endpoint using the production APIs, and calls libc::_exit
without Drops. Parent uses the real owned wait, closes exact endpoint records,
commits final console and Adopt retention, then removes only runtime targets.
The same fixture also covers normal explicit close equivalence. It has no test
issuer, production namespace override, HOME/env mutation or guessed retirement.

Negative authored controls include parent and unrelated-import reads, surviving
runtime read guard, a real forked fixture descendant retaining OFDs until its
private pipe closes, independently held endpoint sentinel, missing Bound socket,
replacement/new runtime/foreign/used receipt, unbound legacy read, ownership epoch
mismatch, old endpoint attribution, pending activity, prevalidated sets and partial
CloseIntent/unlink/fsync/first-record fault boundaries. Existing no-activation and
F/E failures remain. Cross-exec helper returns without consuming stdin during an
ordinary full-suite invocation and is counted separately, not a lifecycle PASS.

Source checks: rustfmt and rustfmt --check on nine edited Rust files exit0;
git diff --check exit0. Bounded Python inspection initially exited1 twice: it
searched for a call in the method-definition file, then assumed a dotted field
expression would not be split by rustfmt. Correcting the assertion locations and
normalizing whitespace passed exit0 without functional-code changes. Final checks
confirm12 owned paths, all113 previous test/helper names retained, unchanged F
begin_remove signatures and explicit ownership at every hold emitter. Lineage,
pidfd, storage and previous standalone test files remain byte-identical to base.

These are AUTHORED_NOTRUN. No Rust compilation, process/descendant/kernel/socket,
filesystem or guest evidence is claimed. POSIX shell + Linux pidfd/OFD, executable
test filter, fchmodat2/getxattrat and filesystem capability remain admitted validation
gates. The shared2GiB reservation and existing worktrees are unchanged. No caller,
Cargo/dependency, CLI, compatibility authority or other repository edit occurred.

Parent must integrate B's runtime resource calls and exact endpoint retirement
before cleanup, restore the positive stop_all lifecycle oracle, then run admitted
combined checks and ONE independent integrated review. No migration8x2, release,
Registry, push/PR or live-host setup is authorized or implied by this source commit.
