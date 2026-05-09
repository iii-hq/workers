# Granting a 24-hour exemption to a principal

## When to use

- Allowing a specific agent or user to bypass the ceiling for a one-off high-cost operation.
- Granting a short-lived exemption during an incident that cannot wait for a budget increase.
- Exempting a system service principal from a workspace budget during a scheduled migration.

## Notes

- Exemptions expire automatically after 24 hours. `budget::check` prunes expired entries on every call, so no background sweep is required.
- Granting an exemption to an already-exempt principal resets the clock to `now + 24h` (upsert, not additive).
- `reason` is required and stored verbatim with the exemption — surface it in audit logs so the next reviewer knows why the bypass exists.
- Exemptions do not modify `spent_usd`; they bypass the ceiling check only.
