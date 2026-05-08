# shell::kill

Terminate a running background job.

`({ job_id }) → { job_id, killed, status, reason? }` — `killed` is `true` when the worker delivered the kill (or marked a sandbox job killed; see notes). `status` is the post-kill `JobStatus`. `reason` is set when the call was a no-op (`"not running"`) or when the kill is advisory (sandbox case).

## When to use

- Cancelling a runaway build or long-running process spawned by `shell::exec_bg`.
- Cleaning up before re-issuing a corrected command.
- Reaping at the end of an orchestration before unregistering the worker.

## Notes

- There is NO `signal` field on the wire. Host kills go through tokio's `Child::start_kill`, which is a hard kill (SIGKILL on Unix).
- A non-`running` job returns `killed: false` with `reason: "not running"`; it is not an error.
- Sandbox-backed jobs cannot be hard-killed because `sandbox::exec` has no cancel hook. The record flips to `killed` and `finished_at_ms` is stamped immediately so `shell::status` / `shell::list` reflect cancellation, but the in-VM process keeps running until its `timeout_ms` expires. The response includes a `reason` explaining this. For real cancellation, set a tight `timeout_ms` on the original `exec_bg`, or call `sandbox::stop` to tear down the VM.
- `not_found` (the trigger `Err`) means the `job_id` either never existed or aged out of retention.
