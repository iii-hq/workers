# auth

Use this worker when an iii project needs one OAuth authority for worker-manager RBAC, MCP bridges, A2A bridges, or generated workers that need bearer-token access.

It is useful when you need to:

- let clients discover auth endpoints without hardcoding URLs
- dynamically register clients at runtime
- issue short-lived RS256 access tokens plus refresh tokens
- validate incoming worker-manager sessions into concrete RBAC decisions
- expose JWKS for local token verification
- introspect or revoke tokens from trusted resource workers

Prefer the smallest function that answers the job:

- `auth::server_metadata` for authorization server discovery
- `auth::resource_metadata` for protected resource discovery
- `auth::register` before a new client can request tokens
- `auth::token` to issue or refresh tokens
- `auth::validate` in worker-manager RBAC middleware
- `auth::jwks` when a verifier needs public signing keys
- `auth::jwks_rotate` for scheduled or manual signing-key rotation
- `auth::introspect` when a trusted resource needs token status
- `auth::revoke` when a client signs out or a token must stop working

Typical flow:

```text
auth::server_metadata -> auth::register -> auth::token -> auth::validate
```

For HTTP clients, the same worker exposes:

```text
GET  /.well-known/oauth-authorization-server
GET  /.well-known/oauth-protected-resource
POST /register
POST /token
GET  /.well-known/jwks.json
POST /introspect
POST /revoke
```

Example client registration:

```json
{
  "client_name": "artifact-worker",
  "scope": "mcp:tools a2a:message"
}
```

Example token request:

```json
{
  "grant_type": "client_credentials",
  "client_id": "client_123",
  "client_secret": "secret_456"
}
```

Example validation request:

```json
{
  "headers": {
    "authorization": "Bearer eyJhbGciOiJSUzI1NiIs..."
  },
  "ip_address": "127.0.0.1"
}
```

Example validation output:

```json
{
  "allowed_functions": ["tools::search"],
  "forbidden_functions": [],
  "allow_function_registration": false,
  "trusted_internal": false,
  "context": {
    "client_id": "client_123",
    "subject": "client_123"
  }
}
```
