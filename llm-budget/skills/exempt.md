# budget::exempt

Grant a principal a 24-hour exemption from budget enforcement.

`({ budget_id, principal_id, reason }) → { budget_id, expires_at: i64 }` — prunes any already-expired
exemptions, removes any existing exemption for `principal_id` (upsert semantics), appends a new
`Exemption` with `expires_at = now + 24h`, and saves the budget. `expires_at` is a Unix
millisecond timestamp.

## When to use

- Allowing a specific agent or user to bypass the ceiling for a one-off high-cost operation.
- Granting a short-lived exemption for an emergency that can't wait for a budget increase.
- Exempting a system service principal from a workspace budget during an incident.

## Notes

- Exemptions expire automatically after 24 hours (`EXEMPT_TTL_MS = 86_400_000 ms`). `budget::check`
  prunes expired exemptions and evaluates only active ones; no background sweep is needed.
- Calling `budget::exempt` for an already-exempt principal resets the clock to `now + 24h`
  (upsert, not additive).
- `reason` is a required free-form string used for audit logging; it is stored with the exemption
  record and returned in `budget::get`.
- An exemption does not accumulate or modify `spent_usd`; it only bypasses the ceiling check.
