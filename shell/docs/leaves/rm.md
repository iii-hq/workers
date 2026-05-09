# Removing a path inside the jail

## When to use

- Tearing down a generated artefact after use.
- Resetting a workspace directory before re-bootstrapping it.
- Dropping a temp file once a flow no longer needs it.

## Notes

- Symlinks are removed by themselves, not their targets. `recursive` does not change that.
- There is no trash bin — this is `unlink(2)` / `rmdir(2)`. Confirm caller intent before invoking.
- `removed` is always `true` on success. Missing paths return `Err(S211)`; non-empty directories without `recursive: true` return `Err(S214)`; permission errors and other I/O failures come back as the trigger `Err` with the corresponding `FsError` shape.
- Same jail and denylist rules as `shell::fs::ls`.
