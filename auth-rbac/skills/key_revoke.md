# auth::rbac::key_revoke

Mark an API key as revoked so it fails all future `verify` calls.

`({ key_id }) → { ok: true }` — sets `revoked_at` on the key record. Idempotent:
revoking an already-revoked key returns `{ ok: true }` without error.
The key record is retained for audit purposes; it is not deleted.

## When to use

- Rotating a compromised or expired API key.
- Offboarding a user or service account from a workspace.
- Retiring keys that have not been used within your key rotation policy window.

## Notes

- `key_id` is the UUID returned by `auth::rbac::key_create` (not the plaintext token).
- Returns an error if no key with the given `key_id` exists.
- Revoked tokens are immediately rejected by `auth::rbac::verify` with reason `"revoked"`.
