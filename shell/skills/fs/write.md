# shell::fs::write

Write a file by streaming bytes through a `ContentRef` channel.

`({ path, content: ContentRef, mode?, parents?, target? }) → { bytes_written, path }` — `content` is a `ContentRef` (`{ channel_id, access_key, direction: "write" }`) that the worker drains until close. `mode` is an octal string (default `"0644"`). `parents: true` creates intermediate directories `mkdir -p` style.

## When to use

- Persist a generated artifact to disk inside the jail.
- Stream a remote download or generated stream straight into a file without an intermediate buffer.
- Bootstrap files into a sandbox by retargeting the call with `target: { kind: "sandbox", sandbox_id }`.

## Notes

- The wire payload does NOT take raw `content: string` or `content_b64`. The caller opens a channel (typically via the SDK's channel APIs), passes the `ContentRef` here, then writes bytes into the channel and closes it.
- When `cfg.fs.max_write_bytes > 0` and the streamed total exceeds the cap, the write is aborted mid-stream with `S218`. The default of `0` means no cap.
- Per-chunk idle timeout is 30 s. If the caller opens a write but never sends data and never closes the channel, the worker aborts with `S216 channel idle for 30s — aborting write` so a parked writer doesn't leak the temp file.
- The worker writes through a temp file and renames atomically. On crash mid-stream the temp file is unlinked by `TempGuard`.
- Approval policy is NOT hardcoded into this function. Whether a turn requires approval before `shell::fs::write` lands is set per-run by the orchestrator's `approval_required` array — operators / harness UIs that pin it there will see a user-approval round-trip; deployments that don't include it write immediately.
- Same jail + denylist rules as `shell::fs::ls`.
