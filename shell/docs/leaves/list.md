# Surveying current background jobs

## When to use

- "What background jobs are running right now?" probes.
- Driving a dashboard or status surface over current shell activity.
- Pre-cleanup audit before a worker shutdown.

## Notes

- The response carries `JobSummary` only — `argv`, `stdout`, and `stderr` are deliberately omitted. The JOBS map is process-wide and any caller could otherwise read another caller's command line and captured output (which may embed credentials).
- For full records (including `stdout`/`stderr`), call `shell::status` with the `job_id`. The random UUID acts as an unguessable capability.
- Terminated jobs stay listed for `cfg.job_retention_secs` (default 3600s) past their `finished_at_ms`. After that the periodic janitor removes them and the list no longer returns them.
