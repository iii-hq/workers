# Creating a directory inside the jail

## When to use

- Pre-creating a directory tree before streaming files into it with `shell::fs::write`.
- Ensure-path-exists idiom: `parents: true` and ignore the result.
- Bootstrapping a sandbox layout by retargeting the call.

## Notes

- The flag is `parents`, not `recursive`. Both backends accept the same name.
- `created` is always `true` on success, including the idempotent case where `parents: true` and the directory already existed. There is no `created: false` branch; failure returns the trigger `Err` (e.g. `S211` for a missing parent without `parents: true`).
- Same jail and denylist rules as `shell::fs::ls`.
