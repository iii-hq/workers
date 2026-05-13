# Renaming or moving a path inside the jail

## When to use

- Renaming in place inside the same parent directory.
- Moving across directories within the jail.
- Atomic publish of a generated file from a temp name to its final location.

## Notes

- Both `src` and `dst` are absolute and must be inside the same jail.
- Implementation is `rename(2)` with a fallback to copy + unlink when `src` and `dst` cross filesystems. The fallback is not atomic on its own; a crash after the copy and before the unlink leaves both copies on disk.
- Without `overwrite: true` the call refuses if `dst` exists and returns the trigger `Err`.
- This is `mv` for one path. There is no batch or glob form; loop in the caller.
- Same jail and denylist rules as `shell::fs::ls` for both `src` and `dst`.
