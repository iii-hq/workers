# auth

OAuth authority worker for iii. It gives MCP, A2A, virtual workers, and normal iii workers one shared auth surface for token issue, validation, discovery, revocation, and worker-manager RBAC.

## Functions

- `auth::validate` validates a Bearer token for `iii-worker-manager` RBAC and returns the session decision shape the engine expects.
- `auth::server_metadata` returns the RFC 8414 authorization server discovery document.
- `auth::resource_metadata` returns the RFC 9728 protected resource discovery document.
- `auth::register` performs RFC 7591-style dynamic client registration.
- `auth::jwks` returns active public signing keys.
- `auth::jwks_rotate` rotates the local signing key and keeps old keys through the overlap window.
- `auth::token` issues client-credentials tokens and rotates refresh tokens.
- `auth::introspect` returns token activity for authenticated clients.
- `auth::revoke` revokes access tokens or refresh tokens.

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

Privileged scopes are intentionally blocked for public registration. Set `III_AUTH_REGISTRATION_TOKEN` and pass it as `Authorization: Bearer <token>` only for internal bootstrap clients that need `function:*`, `trigger:*`, or `iii:*` scopes.

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

Refresh a token:

```json
{
  "grant_type": "refresh_token",
  "client_id": "<client_id>",
  "client_secret": "<client_secret>",
  "refresh_token": "<refresh_token>"
}
```

The old refresh token is revoked and the response includes a new one.

Revoke a token:

```json
{
  "client_id": "<client_id>",
  "client_secret": "<client_secret>",
  "token": "<access_or_refresh_token>",
  "token_type_hint": "access_token"
}
```

## Configuration

```yaml
environment: "local"
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
token_endpoint_auth_methods_supported:
  - "client_secret_post"
  - "client_secret_basic"
registration_admin_token_env: "III_AUTH_REGISTRATION_TOKEN"
state_timeout_ms: 5000
```

Privileged scopes are opt-in. Add them only for deployments that need worker-manager bootstrap authority, and protect registration with `III_AUTH_REGISTRATION_TOKEN`:

```yaml
supported_scopes:
  - "mcp:tools"
  - "a2a:message"
  - "function:*"
  - "iii:function_registration"
  - "iii:trigger_type_registration"
  - "iii:trusted_internal"
```

`idp_mode: local` issues and validates local RS256 JWTs. The worker fails closed if its config file cannot be loaded. The iii state store uses bounded timeouts so auth paths do not wait forever on state.

Set `environment: "production"` or `III_AUTH_ENV=production` to reject insecure `ws://` and `http://` endpoints at startup.

The registry default uses an HTTPS issuer placeholder. Replace it with the real HTTPS authority and certificate for any shared, remote, or production deployment.

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
