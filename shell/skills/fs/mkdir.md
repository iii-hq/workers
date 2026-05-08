# shell::fs::mkdir

Create a directory inside the jail.

`({ path, mode?, parents?, target? }) → { created }` — `path` is absolute. `mode` is an octal string (default `"0755"`). `parents: true` is the `mkdir -p` shape (succeeds if the directory and its intermediates already exist). Without `parents`, the call fails on the first existing path component.

## When to use

- Pre-create a directory tree before streaming files into it with `shell::fs::write`.
- Standard "ensure path exists" idiom: `parents: true` + ignore the result.
- Bootstrap a sandbox layout by retargeting the call.

## Notes

- The flag is `parents` (NOT `recursive`). Both backends accept the same name.
- `created` is always `true` on success — including the idempotent case where `parents: true` and the directory already existed. There is no `created: false` branch; failure returns the trigger `Err` (e.g. `S211` for a missing parent without `parents: true`, I/O errors otherwise).
- Same jail + denylist rules as `shell::fs::ls`.
