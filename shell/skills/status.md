# shell::status

Fetch the full record for a background job, including buffered stdout/stderr.

`({ job_id }) → { job: JobRecord }` where `JobRecord` is `{ id, argv, started_at_ms, finished_at_ms?, status, exit_code?, stdout, stderr, stdout_truncated, stderr_truncated }`. `status` is one of `running`, `finished`, `killed`, `failed`. Time fields are epoch milliseconds.

## When to use

- Polling a job spawned by `shell::exec_bg` until it leaves the `running` state.
- Fetching captured output once a job has terminated, before retention expires.
- Diagnosing a job that exited with a non-zero `exit_code`.

## Notes

- `not_found` (the trigger `Err`) means the `job_id` either never existed or aged out of `cfg.job_retention_secs` (default 1 hour after termination). Don't retry — re-run `shell::exec_bg` if the work still needs doing.
- Per-stream output buffer is bounded by `cfg.max_output_bytes` (default 1 MiB). Once the cap is hit on a stream, the corresponding `*_truncated` flag stays `true` and new bytes are dropped — the job keeps running.
- Use `shell::list` for a lightweight overview of every job. `shell::status` returns the full record (including potentially large stdout/stderr buffers) so it costs more per call.
- Sandbox-backed jobs that were `shell::kill`-ed flip to `killed` immediately even though the in-VM process may still be running; their final stdout/stderr arrive on the late `sandbox::exec` response and are NOT applied if the record is already `killed`.
