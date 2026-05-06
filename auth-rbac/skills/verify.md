# auth::rbac::verify

Validate a plaintext API token, optionally asserting workspace membership and minimum role.

`({ token, workspace_id?, required_role? }) → { valid: bool, key_id?, workspace_id?, role?, reason? }` —
hashes the token and looks it up by HMAC. If `workspace_id` is provided, confirms the key belongs to that
workspace. If `required_role` is provided, confirms the key's role satisfies the hierarchy
(`owner > admin > member > viewer`). Returns `{ valid: false, reason }` for any failure
rather than an error, so callers can branch on the `valid` field directly.

## When to use

- An API gateway or middleware authenticating an incoming request.
- Checking that a service token has at least `admin` role before allowing a privileged operation.
- Validating a webhook caller's token against a specific workspace.

## Notes

- `reason` values: `"missing token"`, `"unknown token"`, `"revoked"`, `"workspace mismatch"`, `"insufficient role"`, or `"invalid required_role: ..."`.
- `last_used_at` is updated in the background at most once per 5 minutes per key to reduce write amplification.
- Timing-safe comparison is used internally; do not try to replicate the verification logic externally.
- Pass neither `workspace_id` nor `required_role` to perform a simple token existence and revocation check.
