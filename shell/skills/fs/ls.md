# shell::fs::ls

List a directory inside the host_root jail.

`({ path, target? }) → { entries: [{ name, is_dir, size, mode, mtime, is_symlink }] }` — `path` is absolute. `mode` is the octal string (e.g. `"0755"`); `mtime` is epoch seconds (NOT milliseconds — `JobRecord.*_at_ms` are ms; `FsEntry.mtime` is seconds). Symlinks are NOT followed; their `is_dir` reflects the link itself and `is_symlink` is `true`.

## When to use

- Enumerate before reading: cheaper than `shell::fs::grep` when you just need filenames.
- Confirming that a path is a directory before recursing.
- Building file pickers / orchestration that needs structured listings.

## Notes

- `path` outside `cfg.fs.host_root` returns an error; the path denylist (`cfg.fs.denylist_paths`) refuses inside the jail too.
- Prefer this over `shell::exec` with `ls`: it stays in-process, returns JSON directly, and respects the jail.
- The wire shape mirrors the engine daemon's `sandbox::fs::ls` response so host and sandbox targets are interchangeable from the caller's point of view.
