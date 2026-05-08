# shell::exec

Run a single command in the foreground and return the full result in one response.

`({ command, args?, timeout_ms?, target? }) → { exit_code, stdout, stderr, duration_ms, timed_out, stdout_truncated, stderr_truncated }` — `command` is the program name as a **string** (matched against the allowlist by basename or exact path); split arguments into `args`, do NOT pass argv as an array in `command`. `args` is an array of strings; non-string elements are rejected by index. `timeout_ms` is clamped to `max_timeout_ms` (default 30 s) and falls back silently to `default_timeout_ms` (default 10 s) on absence or unparseable values (including negative integers and floats). `target` defaults to `{ kind: "host" }`; pass `{ kind: "sandbox", sandbox_id }` to forward the call to a live microVM.

The wire shape is published as a JSON schema via `ExecRequest` in `functions::types`, so the engine's tool listing tells callers each field's type up front.

## When to use

- The command finishes well under the timeout cap and you want its output now.
- One-shot probes: `ls`, `cat`, `pwd`, `git status`, `wc`, `head`, etc.
- Anything where blocking until completion is fine for the calling turn.

## Notes

- Output is buffered in memory up to `max_output_bytes` (default 1 MiB). Beyond the cap, `stdout_truncated` / `stderr_truncated` flip to `true` and the rest is dropped — re-run with a tighter command (e.g. `head -n 100`) instead of asking for more bytes.
- `cwd` and `env` are NOT part of the wire payload. The working directory comes from `cfg.working_dir`; environment is built from `cfg.allowed_env` plus `cfg.inherit_env` at config time.
- Wrong-type fields produce actionable errors via per-field deserializers in `functions::types`. The most common one — sending `command: ["sh", "-lc", "..."]` — returns `'command' must be a string (got array). Pass the program name in 'command' and arguments in 'args', e.g. {"command": "sh", "args": ["-lc", "ls -la"]}` rather than a misleading "missing 'command'".
- Allowlist is matched by `argv[0]`'s basename OR the exact string in `cfg.allowlist`; an empty allowlist means "open." Denylist is a regex set over `argv.join(" ")`. Both refusals come back as the trigger `Err`.
- `target: sandbox` returns `S300` if the host can't boot microVMs (Apple Silicon or `/dev/kvm` required). Host-side spawn errors come back as `S216` with a `host exec:` prefix. In-VM execution failures arrive as `S200`; a recovered `S200` with a "timed out" message is reported as `timed_out: true`.
- Use `shell::fs::ls`, `shell::fs::stat`, `shell::fs::grep` instead of `exec`-ing `ls`/`stat`/`grep`/`rg` — the fs backends stay in-process, respect the jail, and don't share the foreground output cap.
