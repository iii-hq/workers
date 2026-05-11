# Running a one-shot command in the foreground

## When to use

- The command finishes well under the timeout cap and the caller can block until completion.
- One-shot probes: `ls`, `cat`, `pwd`, `git status`, `wc`, `head`.
- Anything where blocking until completion is fine for the calling turn.

## Notes

- `command` is a program name as a string; arguments go in the `args` array. Sending `command: ["sh", "-lc", "..."]` returns a per-field deserialiser error rather than a misleading "missing 'command'".
- `timeout_ms` is clamped to `max_timeout_ms` (default 30s); negative or non-numeric values fall back to `default_timeout_ms` (default 10s).
- Output is buffered up to `max_output_bytes` (default 1 MiB). Past that, `stdout_truncated` and `stderr_truncated` flip to `true` and the rest is dropped — narrow the command (e.g. `head -n 100`) rather than asking for more bytes.
- Allowlist matches `argv[0]` by basename or exact path; an empty allowlist means open. Denylist regex runs over `argv.join(" ")`.
- `target: sandbox` returns `S300` if the host cannot boot microVMs (Apple Silicon or `/dev/kvm` required). Host-side spawn errors come back as `S216` with a `host exec:` prefix.
- Prefer `shell::fs::ls`, `shell::fs::stat`, and `shell::fs::grep` over `exec`-ing `ls`/`stat`/`grep`/`rg` — the fs backends stay in-process and respect the jail.
