# auth::jwks_rotate

Use this for scheduled or manual signing-key rotation.

Trigger: cron from `rotation_cron`, or direct function call when an operator needs rotation immediately.

Input:

```json
{}
```

Sample output:

```json
{
  "ok": true,
  "current_kid": "new-key-id",
  "retained_keys": 2
}
```

The worker keeps the previous key through `rotation_overlap_seconds` so existing access tokens can still verify. Run `auth::jwks` after rotation if a verifier needs the latest public key set.
