# shell::fs::sed

Find-and-replace inside the jail. Operates on an explicit file list, a single file, or a directory walk.

`({ pattern, replacement, files?, path?, recursive?, regex?, first_only?, ignore_case?, include_glob?, exclude_glob?, target? }) → { results: [{ path, replacements, success, error? }], total_replacements }` — supply EITHER `files: ["/abs/a", "/abs/b", …]` for an explicit list OR `path` (a directory) for a recursive walk; or `path` to a single file for a one-off rewrite. `regex` defaults to `true` (literal mode is off). `recursive` defaults to `true` when `path` is a directory.

## When to use

- Multi-file rewrite where `shell::fs::grep` would be the read-side preview.
- Single-file regex replace where you'd otherwise reach for `sed -i`.
- Templating step over a generated tree.

## Notes

- `pattern` is a regex when `regex: true` (default). `replacement` supports `$1`, `$2`, … capture-group backreferences. Set `regex: false` for a literal pattern.
- `first_only: true` rewrites only the first match per file (the equivalent of `occurrences: "first"`); the default is to rewrite all matches.
- There is no line-anchor flag — encode it inside the pattern (`(?m)^…`).
- Per-file results carry `success: false` plus an `error` string for files that failed to rewrite (e.g., permission denied, regex compilation error). `total_replacements` sums across files.
- Approval policy mirrors `shell::fs::write`: NOT hardcoded. Deployments that pin `shell::fs::sed` into the orchestrator's `approval_required` see a user-approval round-trip; others write immediately. Confirm against `workers/turn-orchestrator/src/run_start.rs` if your deployment has tightened the policy.
- Same jail + denylist rules as `shell::fs::ls`. If you're unsure about the regex, run `shell::fs::grep` with the same pattern first.
