# auth

OAuth authority worker for iii. It gives MCP, A2A, virtual workers, and normal iii workers one shared auth surface instead of each protocol worker shipping its own middleware.

## Functions

- `auth::validate` validates a Bearer token for `iii-worker-manager` RBAC and returns the session decision shape the engine expects.
- `auth::server_metadata` returns the RFC 8414 authorization server discovery document.
- `auth::resource_metadata` returns the RFC 9728 protected resource discovery document.
- `auth::register` performs RFC 7591-style dynamic client registration.
- `auth::jwks` returns active public signing keys.
- `auth::jwks_rotate` rotates the local signing key and keeps old keys through the overlap window.
- `auth::token` issues client-credentials tokens, refreshes tokens, and introspects access tokens.

## Install

```bash
iii worker add auth
```

Then point `iii-worker-manager` RBAC at `auth::validate`:

```yaml
workers:
  - name: iii-worker-manager
    config:
      rbac:
        auth_function_id: auth::validate
        expose_functions:
          - metadata:
              public: true
  - name: auth
    config:
      issuer: https://api.example.com
      idp_mode: local
```

## Quickstart

Register a client:

```json
{
  "client_name": "local-mcp-client",
  "grant_types": ["client_credentials", "refresh_token"],
  "scope": "mcp:tools"
}
```

Call `auth::register` with that payload. The response includes `client_id` and, for confidential clients, a one-time `client_secret`.

Issue a token:

```json
{
  "grant_type": "client_credentials",
  "client_id": "<client_id>",
  "client_secret": "<client_secret>",
  "scope": "mcp:tools"
}
```

Call `auth::token`. Use the returned Bearer token when connecting to the worker manager, MCP, or A2A bridge.

## Configuration

```yaml
engine_url: "ws://127.0.0.1:49134"
issuer: "https://api.example.com"
audience: "iii"
idp_mode: "local"
store: "iii_state"
access_token_ttl_seconds: 900
refresh_token_ttl_seconds: 2592000
rotation_overlap_seconds: 86400
default_scopes: ["mcp:tools"]
supported_scopes:
  - "mcp:tools"
  - "a2a:message"
  - "function:*"
  - "iii:function_registration"
  - "iii:trigger_type_registration"
  - "iii:trusted_internal"
```

`idp_mode: local` issues and validates local RS256 JWTs. Bridge modes are advertised in metadata and the IdP matrix so deploy authors can see whether their IdP supports DCR before committing to it.

## IdP Matrix

| IdP | DCR | Metadata | PKCE | Notes |
|---|---|---|---|---|
| Keycloak | yes | yes | required | Best reference bridge target. |
| Okta | yes | yes | required | Good DCR support. |
| Auth0 | yes | yes | required | Good DCR support. |
| Entra ID | no | yes | required | Pre-register clients. |
| Google | no | yes | required | Pre-register clients. |
| Ping | yes | yes | required | Good DCR support. |
| ForgeRock | yes | yes | required | Good DCR support. |
