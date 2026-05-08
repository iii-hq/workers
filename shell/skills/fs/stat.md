# shell::fs::stat

Return a single `FsEntry` for one path.

`({ path, target? }) → { name, is_dir, size, mode, mtime, is_symlink }` — same shape as one entry of `shell::fs::ls`. The response is serialized as a transparent wrapper, so the entry's fields appear at the top level of the result.

## When to use

- Confirm a file exists and check its size before deciding whether to read it whole.
- Distinguish a regular file from a symlink without dereferencing it.
- Pre-flight check for a write target's existing mode before `chmod`-ing.

## Notes

- A missing path returns the trigger `Err` (FsError) — there is no "not_found is normal" envelope. Wrap the call if you want soft probing.
- Symlinks are not followed; `is_symlink` reports the link itself. If you need the target's metadata, follow with another `stat` on the resolved path.
- Same jail + denylist rules as `shell::fs::ls`. `mode` is an octal string and `mtime` is epoch seconds (NOT ms).
