# auth

Use auth as the shared OAuth authority for iii workers, MCP bridges, A2A bridges, and worker-manager RBAC.

- [`auth`](iii://auth)
  - [`auth::validate`](iii://auth/validate): validate Bearer tokens for worker-manager RBAC.
  - [`auth::server_metadata`](iii://auth/server_metadata): read authorization server discovery.
  - [`auth::resource_metadata`](iii://auth/resource_metadata): read protected resource discovery.
  - [`auth::register`](iii://auth/register): dynamically register OAuth clients.
  - [`auth::jwks`](iii://auth/jwks): read public signing keys.
  - [`auth::jwks_rotate`](iii://auth/jwks_rotate): rotate signing keys.
  - [`auth::token`](iii://auth/token): issue, refresh, or introspect tokens.
