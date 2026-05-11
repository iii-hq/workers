# Reading a single path's metadata

## When to use

- Confirming a file exists and checking its size before deciding whether to read it whole.
- Distinguishing a regular file from a symlink without dereferencing it.
- Pre-flight check for a write target's existing mode before `chmod`-ing.

## Notes

- A missing path returns the trigger `Err` (FsError) — there is no soft-not-found envelope. Wrap the call when probing optionally.
- Symlinks are not followed; `is_symlink` reports the link itself. To get the target's metadata, follow up with another `stat` on the resolved path.
- Same jail and denylist rules as `shell::fs::ls`. `mode` is an octal string and `mtime` is epoch seconds.
