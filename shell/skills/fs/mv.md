# shell::fs::mv

Move or rename a path inside the jail.

`({ src, dst, overwrite?, target? }) → { moved }` — both paths are absolute and must be inside the same jail. Without `overwrite: true` the call refuses if `dst` exists.

## When to use

- Rename in place (same parent dir).
- Move across directories within the jail.
- Atomic publish of a generated file from a temp name to its final location.

## Notes

- Implementation is `rename(2)` with a fallback to copy + unlink when `src` and `dst` cross filesystems. The fallback is NOT atomic on its own — a crash after the copy and before the unlink leaves both copies on disk.
- `moved: true` means the operation succeeded. A pre-existing `dst` without `overwrite: true` returns the trigger `Err`.
- This is `mv` for one path. There is no batch / glob form; loop in the caller.
- Same jail + denylist rules as `shell::fs::ls` for both `src` and `dst`.
