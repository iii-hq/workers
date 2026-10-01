# auth::server_metadata

Use this when a client, bridge, or generated worker needs OAuth discovery for the auth worker.

HTTP route: `GET /.well-known/oauth-authorization-server`

Input:

```json
{}
```

Sample output:

```json
{
  "issuer": "http://127.0.0.1:3111",
  "token_endpoint": "http://127.0.0.1:3111/token",
  "registration_endpoint": "http://127.0.0.1:3111/register",
  "jwks_uri": "http://127.0.0.1:3111/.well-known/jwks.json",
  "introspection_endpoint": "http://127.0.0.1:3111/introspect",
  "revocation_endpoint": "http://127.0.0.1:3111/revoke",
  "grant_types_supported": ["client_credentials", "refresh_token"],
  "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
  "scopes_supported": ["mcp:tools", "a2a:message"],
  "idp_mode": "local"
}
```

Use this instead of hardcoding URLs. The worker builds endpoint URLs from the configured issuer.
