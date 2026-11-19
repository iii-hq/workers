# Persistent host admission (0.5 migration candidate)

This branch adds an explicit local administration surface. Its bounded local
validation is recorded below; it is **not** a validated release or a completed
0.3-to-0.5 migration. Full VM, recovery and migration campaigns remain required.
No package or Registry publication is implied. The administrative commands below
are operator instructions, not actions run by this change.

## Before enabling the adapter

The coordination catalog must persist outside every guest/source export at:

```text
/var/lib/iii/worker-admission/v1/<numeric-effective-uid>
```

There is no project-local, environment, HOME, temporary-directory or previous-catalog
fallback. Ordinary startup does not create this namespace or repair permissions.
Missing/insecure installation refuses before protected effects. Provision once for
each effective uid that will run the adapter; that uid is not the guest uid.

An administrator must first approve host storage, ownership and isolation. Then,
using the intended adapter binary (replace 1000 with the actual adapter uid):

```sh
sudo /path/to/sandbox-libkrun admission bootstrap --uid 1000 --apply
# Run as the actual adapter account, NOT as root on its behalf:
/path/to/sandbox-libkrun admission check
```

Bootstrap requires effective uid 0. It uses the fixed /var/lib ancestry, root-owned
administrative parents and a private 0700 per-uid namespace. Existing invalid or
partially provisioned directories refuse: no recursive chmod, delete/recreate or
automatic cleanup. Preserve evidence and diagnose the failure rather than retrying
blindly. Bootstrap is not migration, writer quiescence or permission to delete data.
`check` validates the current effective uid's installation/catalog; it does not prove
KVM, kernel/ACL capabilities, runtime behavior or guest consistency.

This candidate's admission/runtime process controls require Linux with glibc,
pidfd/OFD semantics and the required descriptor-relative filesystem operations.
Managed-cache permission access additionally depends on actual getxattrat/fchmodat2
and ACL semantics (getxattrat first appeared in Linux 6.13). A kernel version string
is not a substitute for testing the operations. macOS release support described in
historical documentation is NOT established for this admission path; there is no
raw-PID or unguarded platform fallback. Unsupported prerequisites are blockers,
not successful skipped tests. Offline migration tests additionally require their
separately admitted noatime/filesystem/syncfs conditions; these commands do not
mount or reconfigure the host to provide them.

## Inspect before an explicit recovery decision

```sh
sudo /path/to/sandbox-libkrun admission inspect --uid 1000
```

The JSON report includes each exact operation_id and record_sha256. It reads current
records; it does not stop writers, signal processes, unlink endpoints or issue a
retirement capability. Protect this output as operational metadata.

After reviewing a particular record and obtaining authorization for its effects,
copy its exact lowercase operation id (32 hex digits) and current digest (64 hex
digits) into ONE selected command:

```sh
sudo /path/to/sandbox-libkrun admission recover --uid 1000 \
  --operation-id <operation-id> --expected-record-sha256 <record-sha256> --apply
sudo /path/to/sandbox-libkrun admission archive --uid 1000 \
  --operation-id <operation-id> --expected-record-sha256 <record-sha256> --apply
```

The angle-bracket values are placeholders, not shell-ready arguments. These are
alternatives with different purposes, not an automatic sequence. Recovery checks
exclusive descriptor fences, identities and durable postconditions; a digest is
compare-and-swap integrity, NOT authorization or proof that a writer is gone.
Unresolved lifetimes, permission restoration, activity writes, missing owned wait
or ambiguous journals remain unresolved. There is no force flag, arbitrary root,
previous-catalog fallback, inferred PID authority, repeat-until-success loop or
remote recovery endpoint. Re-inspect after any error; do not reuse a stale digest.

Archive accepts only proven completed records, durably preserves the evidence bundle
and index before retiring the live row, and does not prune guest data or evidence.
Capacity bounds may refuse; removing archives to bypass them is not authorized by
this command. Neither recovery nor archive supplies guest flush/provenance or a
legacy/restart destroy capability. A consumed ordinary-destroy receipt does not
become reusable for a later prune.

## Results and exit codes

Successful commands print JSON to stdout. Recovery/archive preserve an unresolved
report on stdout and return a nonzero code; they never print an unresolved report
while claiming successful recovery. I/O/authorization failures go to stderr.

| Code | Meaning |
| --- | --- |
| 0 | Requested check/inspection/provisioning completed, or recovery/archive report is completed |
| 1 | Authorization, unsupported platform or operation/I/O failure; preserve partial-state evidence |
| 2 | Invalid CLI arguments (including absent required --apply or CAS fields) |
| 3 | Recovery/archive report remains unresolved or unrecognized |

`--apply` is mandatory for mutation commands and is a local deliberate-action gate,
not elevation or a replacement for operator authorization. Inspection/check never
run recovery automatically. With no subcommand, the adapter still serves normally.

## Remaining validation and migration gates

The following checks establish only the bounded local scope, not full libkrun VM
boot, guest consistency, production deployment, or the entire authored test suite.

### Recorded local validation

Tested code commit: `283cf2f2defdb4f2ef83dbc56583a0e9ee631321`.
The documentation-only delivery commit preserves those Rust/Cargo source bytes.
Rust 1.97.1, Linux, existing dependencies, shared target; no downloads or host
provisioning were performed. The build command was:

```sh
cargo +1.97.1 test --locked --offline -p sandbox-libkrun --lib \
  --no-default-features --no-run -j16
```

The library test executable SHA-256 is
`ef714fe1d1711bb5f82f89d65f2ce96bd66de394a7a92e3d7fbc10c0ffdf83bd`.
Fourteen exact tests then passed sequentially, with zero failures or ignored tests:

- Short catalog mutex contention waits, while lifetime sentinels still refuse
  nonblocking conflicts.
- Four concurrent sandbox creates produce two successes and two `S400` results
  with a shared cache/context and real owned fixture children.
- Shared warm-cache readers perform no diagnostic/inode writes; missing inputs
  and pinned metadata do not become cache hits; surviving stage FDs block commit.
- Actual child wait plus exact endpoint settlement handles normal close and
  `_exit`; missing/replaced endpoints, host-drain failure and abandoned children
  do not grant cleanup authority.
- Repeated build planning with a new warm/cold base reference retains the original
  lifecycle; missing/foreign/used/stale receipts preserve the earlier cache data.
- Ordinary destroy retains guest data; an explicitly requested initial prune is
  scoped; a consumed destroy receipt cannot authorize a later prune.
- `stop_all` completes owned child shutdown, removes runtime state and leaves
  private endpoint parents retained.

These are filesystem/Unix-socket/owned-process fixtures, **not guest VM campaigns**.
The cold-cache fixture supplies deterministic local bytes instead of pulling from
a registry. An earlier exact nine-test CLI parser/validation/report run passed on
`3bed9c55a2b685dcf66112ec5c848b28acfbda80`; the relevant CLI source bytes are unchanged.
Those tests do not execute bootstrap, recovery or archive administration.

Initial compile errors and fixture failures were preserved, corrected and retested.
The concurrent-create test exposed a production defect: the short catalog mutex
incorrectly used the nonblocking lifetime-lock helper. It now serializes catalog
transactions; operation/runtime/endpoint sentinel semantics are unchanged. One
failed fixture produced late stderr after its parent exited; both recorded hashes
were retained and failure paths now wait for owned children and drain their output.
All final-round log hashes were stable after exit. No test timeout, capacity oracle
or safety check was relaxed. Compiler warnings remain recorded; no full Clippy or
all-features pass is claimed.

### Explicit unfinished scope

- The protected non-overlay clone of unreadable (`0000`) source trees and faithful
  sealed OCI tree publication remain incomplete and are not validated here. The
  separate partial clone implementation is not included in this branch. No chmod
  under shared Strict claims or lossy metadata fallback is permitted. A new clone
  of an already-shared unreadable source without a retained readable capability
  may explicitly refuse/defer; that limited behavior is not successful cloning.
- Legacy/restart durable handoff, a later prune after a consumed destroy receipt,
  private endpoint-parent reclamation, comprehensive ACL/xattr/platform validation,
  performance, and full crash/descendant coverage remain pending.
- Real guest boot/rebuild, authenticated old-state authority/provenance, writer
  retirement, sanitization, guest-consistent snapshots and the eight-origin by
  two-campaign migration matrix are separate unfinished gates. The Compose
  integration and final coordinated smoke tests are not established by this PR.

No `/var/lib` provisioning, mounts, `syncfs`, external network, production VM or
signals to existing unrelated processes were authorized by these local checks.
Test signals targeted only owned fixture children through stable pidfds. Do not
label the full migration ready, auto-deploy this candidate or publish 0.5 to make
missing Registry-dependent checks appear green.
