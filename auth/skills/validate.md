# auth::validate

Validate an incoming worker-manager session. Pass the RBAC payload with `headers`, `query_params`, and `ip_address`; the function returns allowed function ids, trigger permissions, context, and trusted-internal status.
