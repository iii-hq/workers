# auth::validate

Use this inside worker-manager RBAC to turn an incoming bearer token into an iii authorization decision.

Input can provide the token through `headers.authorization` or compatible request metadata. Include `ip_address` when available so loopback/internal policies can be evaluated consistently.

Input:

```json
{
  "headers": {
    "authorization": "Bearer eyJhbGciOiJSUzI1NiIsImtpZCI6..."
  },
  "query_params": {},
  "ip_address": "127.0.0.1"
}
```

Sample output for a normal client:

```json
{
  "allowed_functions": ["tools::search"],
  "forbidden_functions": [],
  "allowed_trigger_types": null,
  "allow_trigger_type_registration": false,
  "allow_function_registration": false,
  "trusted_internal": false,
  "context": {
    "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
    "subject": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
    "scope": "function:tools::search"
  }
}
```

Sample output for a privileged internal client:

```json
{
  "allowed_functions": ["*"],
  "forbidden_functions": [],
  "allowed_trigger_types": ["http"],
  "allow_trigger_type_registration": true,
  "allow_function_registration": true,
  "trusted_internal": true,
  "function_registration_prefix": null,
  "context": {
    "client_id": "worker-manager",
    "subject": "worker-manager"
  }
}
```

Reject the session if this function errors, returns an inactive decision, or lacks the function/trigger permission required by the requested operation.
