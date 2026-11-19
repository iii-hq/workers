# F: process control, reap observation and exact cleanup receipts

SOURCE implementation incremental after `4202dc282f751cb8c85534f2b0dd9d332e057616`.
This document is the concrete consumer seam. No syscall fixture, process, signal,
VM, Rust compilation or test was executed in this delivery. E1/E2/E4 stay intact.
Status is not authority; neither PID, ExitStatus, bool, JSON nor a caller callback
can construct RuntimeControl, WriterRetirement or NeverSpawned.

## Actual crate-private signatures

```rust
// Existing prepare_runtime remains unchanged.
impl RuntimeLaunch {
    fn spawn(self, command: tokio::process::Command)
        -> Result<RuntimeChild, RuntimeSpawnError>;
    fn cancel_before_spawn(self) -> io::Result<NeverSpawned>;
}
impl RuntimeSpawnError {
    fn error(&self) -> &io::Error;
    fn into_parts(self) -> (io::Error, Option<RuntimeChild>);
}
impl RuntimeChild {
    fn control(&self) -> RuntimeControl;
    fn id(&self) -> Option<u32>; // diagnostic only
    fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin>;
    fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout>;
    fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr>;
    async fn wait(self) -> RuntimeCompletion;
}
#[derive(Clone)] struct RuntimeControl;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeSignal { Terminate, Kill }
impl RuntimeControl {
    fn signal(&self, signal: RuntimeSignal) -> io::Result<()>;
    fn observation(&self) -> RuntimeObservation;
    fn subscribe(&self) -> RuntimeObserver;
    async fn terminal(&self) -> RuntimeObservation;
}
impl RuntimeObserver {
    fn observation(&self) -> RuntimeObservation;
    async fn changed(&mut self) -> RuntimeObservation;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimePhase {
    Running, Reaped, WaitFailed, Abandoned, RetirementFailed, Retired
}
impl RuntimeObservation {
    fn phase(&self) -> RuntimePhase;
    fn os_status(&self) -> Option<std::process::ExitStatus>;
    fn failure(&self) -> Option<&RuntimeFailure>;
    fn retirement(&self) -> Option<&WriterRetirement>;
    fn is_terminal(&self) -> bool;
}
impl RuntimeCompletion {
    fn os_status(&self) -> Option<std::process::ExitStatus>;
    fn retirement(&self) -> Result<&WriterRetirement, &RuntimeFailure>;
}
impl RuntimeFailure { // Clone, Debug, Display, Error; bounded message
    fn kind(&self) -> io::ErrorKind;
    fn raw_os_error(&self) -> Option<i32>;
    fn to_io_error(&self) -> io::Error;
}
#[derive(Clone)] struct WriterRetirement; // private, shared one-shot use
struct NeverSpawned;                     // private, not Clone
impl WriterRetirement {
    fn begin_remove(&self, lease: &Lease, root: &Path, targets: &[&Path])
        -> io::Result<MutationGuard>;
}
impl NeverSpawned {
    fn begin_remove(&self, lease: &Lease, root: &Path, targets: &[&Path])
        -> io::Result<MutationGuard>;
}
```

`Lease::begin(MutationPlan::Remove { .. })` now refuses. Only these receipt methods
enter the existing guarded Remove state machine. They do NOT add an unchecked
cleanup implementation. Constructors and issuing functions remain private to the
core; no API accepts caller-supplied proof metadata.

## Spawn/control/wait example for BOTH consumers

Illustrative control flow; caller policy/auth and error types remain caller-owned.
The real command must already include the existing admission FD handoff, and the
boot process must import the runtime FD before writer effects as in D/E.

```rust
let launch = admission.prepare_runtime(&runtime_paths)?;
let mut child = match launch.spawn(command) {
    Ok(child) => child,
    Err(error) => {
        let (cause, owned_child) = error.into_parts();
        if let Some(child) = owned_child {
            // POST-SPAWN handle-acquisition failure: retain/report this control,
            // attach actual wait before returning the startup error. No cleanup.
            let control = child.control();
            attach_to_instance(control); // caller bookkeeping, not proof issuer
            tokio::spawn(async move {
                let completion = child.wait().await;
                report_start_failure_and_completion(cause, completion);
            });
        } else {
            // May be preflight or OS spawn/pre_exec failure. NOT rollback proof.
            report_start_failure(cause);
        }
        return startup_failed();
    }
};
let control = child.control();
let stdin = child.take_stdin();
let stdout = child.take_stdout();
let stderr = child.take_stderr();
attach_to_instance(control.clone());
// Attach ownership BEFORE any fallible post-spawn await (including stdin send).
let completion_task = tokio::spawn(async move { child.wait().await });
feed_stdin_and_pump(stdin, stdout, stderr).await; // errors cannot drop the Child

// Stop uses the kernel identity, NEVER diagnostic id()/raw kill.
control.signal(RuntimeSignal::Terminate)?;
// Existing caller grace policy may wait for observation or a timeout.
// Do not infer successful stop from a sent signal or caller claim flag.
if control.observation().os_status().is_none() {
    control.signal(RuntimeSignal::Kill)?;
}
let completion = completion_task.await?;
let os_status = completion.os_status(); // available even on catalog failure
let retirement = completion.retirement().map_err(RuntimeFailure::to_io_error)?;
finish_pumps_control_files_and_endpoint_close().await?;
// Declare exact mutable layout roots/files before launch. No broad new roots.
let mut removal = retirement.begin_remove(&admission, &layout, &[&layout])?;
removal.remove_path(&layout)?;
removal.commit()?;
remove_registry_only_after_commit();
```

If stop races reap, signal can return a refusal or ESRCH. Handle that against the
core observation; do not fall back to PID or turn retirement failure into stopped
success. Watch snapshots may coalesce Reaped into RetirementFailed/Retired; every
later observation retains os_status. `subscribe` deliberately returns an owned
observer wrapper, not a borrowable watch value that could deadlock signal/reap.

## Why the PID-reuse window is closed in source

A private SOCK_SEQPACKET/CLOEXEC socketpair is created before spawn. In the child's
pre_exec the core calls `pidfd_open(getpid(), 0)` and transfers that pidfd using
SCM_RIGHTS. The child is necessarily alive at this acquisition, even if it exits
immediately after exec; the parent NEVER opens a pidfd from a potentially stale
post-wait numeric PID. Only libc named SYS constants are used, no guessed numbers.
The child closes handoff FDs; the parent receives CLOEXEC. No guest endpoint/env
or production routing override is involved. Unsupported Linux pidfd/socket semantics
fail explicitly with no raw-kill fallback.

On receive failure AFTER successful spawn, RuntimeSpawnError owns the real
RuntimeChild. Its control refuses signals without the stable handle, but wait and
observation remain usable. Dropping this error transfers the child to the same
conservative core reaper path; it is not proof that no child existed. Any OS
spawn error preserves uncertain runtime evidence and retained descriptors.

RuntimeControl.signal uses only pidfd_send_signal. Its bounded state mutex is
never held across Child.wait/catalog I/O/await. Actual successful OS wait publishes
Reaped and drops signalability BEFORE failed_ofd, sentinel acquisition, identity
checks or catalog persistence. A concurrently executing signal targets the original
kernel process even in the tiny interval before publication, never a reused PID.
The code does not claim OS reap equals writer retirement or guest consistency.

## Error, cancellation and lifetime rules

- Wait error: WaitFailed, os_status=None; not reap. Stable pidfd remains safe for
  stop while original process might still run. No receipt.
- Post-reap finalization error: RetirementFailed with actual os_status; signal
  permanently refused; original operation safety holds retained. No receipt.
- A descendant retaining the runtime OFD prevents retirement even after wait.
  Activity Pending/Failed, the shared failure cursor, substituted roots and catalog
  errors still refuse. Only validated commit issues WriterRetirement.
- Waiter cancellation/drop publishes terminal Abandoned synchronously, so stop
  observers do not wait forever on an abandoned notifier. Child ownership transfers
  to a core Tokio reaper when a runtime exists. Later real reap updates os_status
  but does not erase abandonment or manufacture retirement. No signal in Drop.
- Without a usable Tokio runtime, Drop conservatively retains the Child/leases
  rather than pretending it reaped. Runtime shutdown can abandon the scheduled
  reaper too; Abandoned remains terminal and unresolved. Explicit recovery is
  required; no startup sweep is authorized.
- Core sets kill_on_drop(false) on the child command to avoid an implicit raw-PID
  signal path during cancellation. This changes implicit cancellation to retention;
  explicit stop still uses the delivered capability and caller's existing grace.
- Observers/receipts retain the operation. Consumer must dispose them after a
  completed cleanup; retention is safe but can delay transitions/resource release.

## Exact cleanup and safe rollback

Receipts retain the same Lease Arc, namespace/operation sentinel, exact declared
entry pins and last runtime epoch. begin_remove rejects another lease/context,
new runtime epoch, changed target/root identity, unresolved holds/journals/endpoints,
unlisted targets and already consumed receipt. Validation and Remove hold acquisition
share the catalog transaction. Targets must be explicitly declared entries, not
arbitrary descendants inferred from a broad root. Completed disjoint logs/endpoints
can finish before cleanup; no generic pin refresh is introduced.

Pre-spawn rollback uses the actual unconsumed launch:

```rust
// authorization + admitted Create/Publish happened in this SAME operation
let launch = admission.prepare_runtime(&[])?; // zero writer footprint for rollback
let rollback = launch.cancel_before_spawn()?; // no spawn call was attempted
let mut removal = rollback.begin_remove(&admission, &new_layout, &[&new_layout])?;
removal.remove_path(&new_layout)?;
removal.commit()?;
```

NeverSpawned only removes exact targets with this operation's committed
Create/Publish-from-absence evidence (or its own completed Remove for an idempotent
repeat). Merely calling local(existing_legacy_path), preparing an empty runtime and
canceling cannot authorize deleting old data. Drop never returns a rollback receipt;
spawn Err cannot be converted to one. No consumer bool/PID/ESRCH/lifeline/sweep is
an alternative. If a layout exists from another operation or before this operation,
retain it for explicit recovery/authorized existing-data policy rather than guessing.

## Source tests and pending gates

All89 prior E tests and their names remain; total100 tests are authored. New
AUTHORED_NOTRUN tests cover actual owned wait
with catalog lock failure, signal/reap pidfd race, wait-error versus reap, abandoned
observer completion, post-spawn pidfd receive failure, missing executable, descendant
OFD retention, real wait/free-sentinel receipt, legacy/stale/foreign/one-shot removal
and explicit pre-spawn rollback. Positive runtime activation uses an explicit own-
context same-process import fixture, NOT a claimed production cross-exec/guest test.
The real child command, pidfd transfer, signal race and cancellation are authored
only; no kernel/process/FS result is asserted.

B must remove UNREAPED_VMS/caller bool authority and route both launcher/waiter/stop,
DirsGuard/rollback, reaper/registry and startup cleanup to these APIs. This core
commit does not edit or claim those routes. Preserve auth/instance/retention policy;
no whole-layout deletion until receipt + Remove commit. F2 source routing and cold/
warm normal lifecycle, kernel pidfd semantics, descendant process lifetime and
first-failure-stop validation remain coordinator gates before the single review.
Source checks on this increment: rustfmt and rustfmt --check exit0, git diff
--check exit0, bounded Python static base/9-owned-path/API/previous-test-name
checks exit0. Compiled0/executed0. These do not establish libc/Tokio/pidfd kernel
compatibility or lifecycle correctness; admitted compile/process tests are pending.
One Workers PR; no release/Registry or remote actions. No compatibility grant,
OLD reimport, guest flush or authenticated historical migration campaign is implied.
