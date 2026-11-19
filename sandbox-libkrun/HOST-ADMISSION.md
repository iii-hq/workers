# Persistent host admission (0.5 migration candidate)

This branch adds an explicit local administration surface. It is source-integrated,
not a validated release: compile, kernel/filesystem, normal lifecycle, recovery and
migration campaigns remain required. No package or Registry publication is implied.
The commands below are operator instructions, not actions run by this change.

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

The source merge is not proof of normal cold/warm/stop/reaper behavior. Compile and
run the admitted tests, including _exit/descendants, exact endpoint settlement,
final log writes and retention. Test failures and refusals stay visible. Legacy
restart/durable handoff, prune after consumed receipt, private endpoint-parent
retention, guest-consistent snapshots and authenticated 8-origin x 2-campaign data
migration are separate unfinished gates. Do not label the full migration ready or
publish 0.5 to make missing Registry-dependent checks appear green.
