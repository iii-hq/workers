# shell::list

Lightweight summary of every background job the worker currently knows about.

`({}) → { jobs: [JobSummary], count }` where `JobSummary` is `{ id, status, started_at_ms, finished_at_ms?, exit_code?, stdout_truncated, stderr_truncated }`. `argv`, `stdout`, and `stderr` are deliberately omitted — the JOBS map is process-wide and any caller could otherwise read another caller's command line and captured output (which may embed credentials).

## When to use

- "What background jobs are running right now?" probes.
- Building a dashboard or status page over current shell activity.
- Pre-cleanup audit before a worker shutdown.

## Notes

- Reachable as the section URI `iii://fn/shell/list` for a humans-readable rendering of the same data.
- For full records (including stdout/stderr), call `shell::status` with the `job_id`. The random UUID in `id` (formatted `job-<uuid>`) acts as an unguessable capability for that record.
- Terminated jobs stay listed for `cfg.job_retention_secs` (default 3600 s) past their `finished_at_ms`; after that they are removed by the periodic janitor and disappear from this list.
