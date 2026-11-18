# Worker lifecycle compatibility admission

This boundary refuses unsafe NORMAL operations; it does not enable migration or
booting an old root. No request/marker/hash/token can issue TransitionAuthority,
BootAuthority or OfflineGrant. All constructors remain private/unimplemented in
production. A valid active selector is still unavailable until authentic local
owner/provenance/retirement/sanitization/profile admission exists.

## Deterministic custody and binding

For adapter base `<parent>/<base-name>` and existing SHA256(state_key)[0..32] vm_id,
compatibility custody is `<parent>/.<base-name>-compat/<vm_id>`. This is outside the
VM directory and guest root, so ordinary destroy/prune cannot remove its records,
candidates, selection or offline data. The full record binding must begin with
that vm_id; its exact full binding and environment are retained by the future
private handoff. The coordinator must use the same path/identity protocol and
resolve aliases/host shares before enrollment; no arbitrary request path is accepted.

Any per-VM custody entry (including empty, corrupt, symlink, wrong owner/mode,
partial staging or missing active selector) refuses. Valid active state is read
through the existing offline Dir/record validators: no-follow chains, private0700
custody,0600 single-link bounded records, version/binding/operation/generation/hash
agreement. Unknown version and malformed records never fall back to normal mode.
Legacy `<vm>/compat`, `compat-mode` and `compat-generation` markers also refuse
without interpreting marker content as authority. Keyless prepare refuses when
an enrollment registry exists, because it cannot identify a safe VM scope.

Sources/build paths and local boot roots pointing inside a generation/backup
(receipt.json/backup.json/active.json or reserved `.<base>-compat` ancestry) also
refuse. This protects retained data from being repurposed as a normal source/upper.
Normal paths with no compatibility intent do not create a marker, state file or
lockfile. Normal build-dir claim/S505 semantics and explicit prune behavior remain;
only participating unsafe custody paths get the existing operation error category
(S300/S301/S507/S400 wrapper), not a new wire code or capability feature.

## Participating callsite matrix

| Entry | Guard before | Lease lifetime |
| --- | --- | --- |
| WorkerVms.prepare | source preparation, rootfs pull/cache/EROFS, state-key directory | request plus blocking EROFS task |
| WorkerVms.start | source mutation, next_instance/Preparing, build claim, adopt_build_mode, scripts/upper | request -> launch waiter -> confirmed child wait |
| WorkerVms.build/rebuild | output claim/create, scratch sweeps, setup/install VM, image publish | request + local VM + blocking image writer |
| WorkerVms.destroy(prune false/true) | Destroying slot, stored-instance read, retain/archive or deletion | entire synchronous mutation |
| WorkerVms.launch direct | next_instance/console/state/process spawn | confirmed child wait |
| local/bundle/binary command builders | firmware/cache, clone/reclone cleanup, workspace/scripts, ensure_base/upper | VmCommand carried lease, owning waiter consumes |
| image_build_command | firmware/rootfs/image/scratch effects | VmCommand and blocking base-image preparation |
| __vm-boot | boot_vm/rootfs mutation after draining stdin env frame | local process never-returning lifetime |
| read/status/logs and authorized stop | unchanged: not boot/capture admission | normal control behavior retained |

The final boot guard also rejects retained lower/upper paths. Source copy-in and
watcher behavior is behind an admitted normal VM lifetime; no compat guest/source
sync is started. Stop remains available to quiesce a live normal VM and is not
misrepresented as credential retirement. Explicit destroy/prune on a compat entry
is refused pending negotiated intent instead of silently retaining/deleting data.

## Shared/exclusive transition fence

Normal operations hold shared flock on every EXISTING ancestor through
`base.parent`, not base itself (base's existing exclusive build-claim lock is
untouched). Direct local worker roots locate the same parent by vm_id ancestry;
other local roots use their own existing ancestor chain. This lets a transition
exclusively lock the final stable parent before creating/enrolling custody and
prevents a check-absence -> marker-created race. If parents are missing, their
existing ancestors are pinned/locked; no directories are created by admission.
The transition API requires a private TransitionAuthority with no producer.

Limits are deliberate: this is coarse parent-wide exclusion, not per-VM concurrency.
A transition may wait/refuse because another VM under the same owner parent is
active. An overly broad base (or missing hierarchy) can conservatively lock a broad
ancestor. Directory-lock-unsupported filesystems fail closed, not warning/fallback.
Normal concurrent operations keep shared locks and existing slot/build mutex rules.
No parent lock is taken on the build-claim base directory, avoiding self-deadlock.

Lease handles are Arc-owned and cross async/blocking-task boundaries. A normal
VM request cannot release them while its child still writes. A successful wait
releases leases; an unknown wait leaks those descriptors conservatively for the
adapter lifetime. A build future dropped after child spawn but before successful
wait likewise retains its leases for that process lifetime, rather than reporting
quiescence while existing kill_on_drop/reaping is uncertain. This can block future
compatibility transitions after a cancelled build until a controlled adapter
shutdown; it does not authorize stopping it, sending new signals or claiming no
writer. Existing child kill/stop semantics were not changed. __vm-boot additionally
holds its own normal guard. Blocking writers retain cloned guards until finished.

Direct command consumers outside WorkerVms must retain the command's private
leases (in-tree owning waiters do); __vm-boot rechecks before effects. A future
transition must still prove all old/uncooperative writers gone. Alternate bases,
unregistered aliases, arbitrary host/guest writes, and old binaries do not become
cooperative through this guard. No all-world or crash-surviving authority claim.

## Private CURRENT handoff

`compat_generation::admission::handoff` takes an existing Store, private
BootAuthority and TransitionAuthority; acquires the exclusive parent lease, calls
Store::selected (current generation, immutable lower validation) and checks full
binding/environment against both proofs. The returned private Handoff retains
Selected/store lease, boot-proof lifetime and transition lease together. No
normal call can construct it or pass it as a bool. It does not contain a guessed
mount profile or execute commands. The offline G1 parent-current-data checks,
G2 syncfs and G3 recovery barriers are untouched.

Remaining seam: local OS-peer issuer, old-writer and token/session/observer
retirement, secret sanitation and actual supported per-VM profile must be supplied
and independently reviewed before any boot/rebuild consumer exists. The tests'
private synthetic issuer is not product authority. Partial refusal is not the
original data-preserving upgrade goal and does not certify a guest migration.

New tests are authored only: guard lifetime/collision/normal behavior, valid and
invalid custody, real request methods refusing before effects, existing admin
semantics without compat, local builders, and CURRENT/NEW handoff. No VM/socket/
process test or offline syncfs was run in this source increment. Historical34
source-approved offline tests remain unchanged and unexecuted.
