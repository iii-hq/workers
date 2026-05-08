# shell::fs::rm

Unlink a file or remove a directory inside the jail.

`({ path, recursive?, target? }) → { removed }` — `path` is absolute. `recursive: false` (the default) refuses non-empty directories. `recursive: true` removes the directory and its contents.

## When to use

- Tear down a generated artifact after use.
- Reset a workspace directory before re-bootstrapping it.
- Drop a temp file once a flow no longer needs it.

## Notes

- Symlinks are removed by themselves, not their targets. `recursive` does NOT change that.
- There is no trash-bin. This is `unlink(2)` / `rmdir(2)`. Get the user's intent right before calling.
- `removed` is always `true` on success — there is no soft-success branch. Missing paths return `Err(S211)`; non-empty directories without `recursive: true` return `Err(S214)`; permission errors and other I/O failures come back as the trigger `Err` with the corresponding `FsError` shape.
- Same jail + denylist rules as `shell::fs::ls`.
