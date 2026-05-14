# auth::register

Use this before a new MCP bridge, A2A bridge, generated worker, or internal client asks `auth::token` for credentials.

HTTP route: `POST /register`

Use public registration for normal scopes such as `mcp:tools` and `a2a:message`. Use an admin bearer token only when registering privileged scopes such as `function:*`, `trigger:*`, or `iii:*`.

Minimal input:

```json
{
  "client_name": "artifact-worker"
}
```

Input with explicit scopes and grants:

```json
{
  "client_name": "artifact-worker",
  "redirect_uris": ["http://127.0.0.1:3000/callback"],
  "grant_types": ["client_credentials", "refresh_token"],
  "scope": "mcp:tools a2a:message",
  "token_endpoint_auth_method": "client_secret_post"
}
```

Privileged registration input:

```json
{
  "headers": {
    "authorization": "Bearer admin-secret"
  },
  "client_name": "worker-manager",
  "scope": "function:* iii:function_registration iii:trusted_internal"
}
```

Sample output:

```json
{
  "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "client_secret": "iN1PqXZJDEU6M5HsR3uHz12vQk1eQJ3TR1T1lPYU6Oc",
  "client_name": "artifact-worker",
  "client_id_issued_at": 1770000000,
  "grant_types": ["client_credentials", "refresh_token"],
  "redirect_uris": [],
  "scope": "mcp:tools a2a:message",
  "token_endpoint_auth_method": "client_secret_post"
}
```

Do not request privileged scopes for public clients. The worker rejects those unless the configured admin token is present.
