# shell::fs::chmod

Change a path's permissions inside the jail. Optionally also change its UID/GID.

`({ path, mode, uid?, gid?, recursive?, target? }) → { updated }` — `path` is absolute. `mode` is an octal string (e.g. `"0644"`, `"0755"`). `uid` and `gid` are optional integers; pass them to also chown. `recursive: true` walks into the directory; default is single-path.

## When to use

- Mark a freshly-written script executable after `shell::fs::write`.
- Lock down a config file (`"0600"`) after generating it.
- Recursively normalise permissions on a generated tree.

## Notes

- `updated` is the count of paths whose mode/owner the call mutated (1 for a single-path call, more under `recursive: true`). Do NOT rely on a `mode_before` / `mode_after` envelope — the legacy docs claimed those fields but the actual response is just `{ updated }`.
- Symlinks are not followed (operates on the link itself).
- `uid` / `gid` are advisory: the host backend will refuse to chown without sufficient privileges; the call returns the trigger `Err` in that case rather than silently skipping.
- Same jail + denylist rules as `shell::fs::ls`.
