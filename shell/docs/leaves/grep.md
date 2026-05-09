# Searching a directory tree with regex

## When to use

- Where a caller would otherwise reach for `rg` or `grep -rn` — faster and structured.
- Pre-step before `shell::fs::sed` to preview what would be rewritten.
- Cross-file search inside a worker without spawning a process.

## Notes

- `include_glob` and `exclude_glob` are arrays of glob strings (e.g. `["**/*.rs"]`, `["**/target/**"]`). The keys are singular `_glob`, not plural `_globs`.
- The flag is `ignore_case`, not `case_insensitive`.
- Multiline mode is off by default; encode `(?m)` inside the pattern when multiline anchoring is needed.
- When `max_matches` is hit, `truncated: true` is set and the walk stops. Tighten the pattern or narrow `path` instead of bumping the cap blindly.
- Binary files are skipped automatically. The wire `FsMatch` has no `column` field; the legacy `file` alias is accepted on the deserialiser side but the response always renders as `path`.
- Same jail and denylist rules as `shell::fs::ls`.
