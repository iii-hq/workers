# auth::resource_metadata

Use this when a protected MCP or A2A resource needs to advertise which authorization server and scopes clients should use.

HTTP route: `GET /.well-known/oauth-protected-resource`

Input:

```json
{}
```

Sample output:

```json
{
  "resource": "iii",
  "authorization_servers": ["http://127.0.0.1:3111"],
  "scopes_supported": ["mcp:tools", "a2a:message"]
}
```

Use `auth::server_metadata` next when the client needs token, registration, JWKS, introspection, or revocation endpoint URLs.
