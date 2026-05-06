# auth-credentials

Provider credential vault. Stores API keys and OAuth tokens for downstream
workers so providers and agents never see raw secrets.

- [`auth-credentials`](iii://auth-credentials)
  - [`set_token`](iii://auth-credentials/set_token) — store a credential
  - [`get_token`](iii://auth-credentials/get_token) — read a credential
  - [`delete_token`](iii://auth-credentials/delete_token) — remove a credential
  - [`list_providers`](iii://auth-credentials/list_providers) — list providers with stored credentials
  - [`status`](iii://auth-credentials/status) — check whether a credential is stored

For workspace API keys and roles, see [`auth-rbac`](iii://auth-rbac).
