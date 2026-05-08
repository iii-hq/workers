# shell::fs::grep

Recursive regex search over a directory inside the jail.

`({ path, pattern, recursive?, ignore_case?, include_glob?, exclude_glob?, max_matches?, max_line_bytes?, target? }) → { matches: [{ path, line, content }], truncated }` — `path` is absolute. `pattern` is a regex evaluated by the backend (Rust regex on the host backend). `recursive` defaults to `true`. `ignore_case` defaults to `false`. `max_matches` defaults to 10,000. `max_line_bytes` defaults to 4096 — longer matched lines are truncated in the `content` field.

## When to use

- You'd reach for `rg` or `grep -rn` from a shell. Faster and structured.
- Pre-step before `shell::fs::sed` to preview what would be rewritten.
- Cross-file search inside a worker without spawning a process.

## Notes

- `include_glob` and `exclude_glob` are arrays of glob strings (e.g. `["**/*.rs"]`, `["**/target/**"]`). Plural names are wrong — use the singular `_glob` keys.
- `ignore_case` (NOT `case_insensitive`) is the correct field name.
- Multiline mode is off by default in the host backend — encode `(?m)` inside the pattern if you need multiline anchoring.
- When `max_matches` is hit, `truncated: true` is set and the walk stops. Tighten the pattern or narrow `path` instead of bumping the cap blindly.
- Binary files are skipped automatically. The wire `FsMatch` has no `column` field; the legacy `file` alias on the wire is accepted by the deserialiser but always rendered as `path` on the response.
- Same jail + denylist rules as `shell::fs::ls`.
