# iii-sandbox-abi

Shared types and stable `S`-code error space for the `sandbox::*` worker
family. Consumed by the `sandbox` router worker and every adapter
(`sandbox-e2b`, `sandbox-daytona`, `sandbox-vercel`, `sandbox-morph`,
`sandbox-modal`, `sandbox-cloudflare`).

Not a worker. Not published to the registry. Pure types.

## Layout

| Item | Purpose |
|---|---|
| `ids::CREATE` / `EXEC` / `STOP` / `LIST` / ... | Caller-facing ids registered by `sandbox` |
| `ids::provider(name, leaf)` | Provider-namespaced id helper (`sandbox::provider::<name>::<leaf>`) |
| `CreateRequest` / `CreateResponse` / `ExecRequest` / ... | Request / response shapes |
| `SCode` / `AbiError` | Stable S-code space and the typed error that emits it |
| `map_http_status` | HTTP status → S-code for provider REST adapters |

## Namespace

Callers use `sandbox::create`, `sandbox::exec`, etc. The `sandbox` router
worker dispatches to `sandbox::provider::<name>::<leaf>` based on the
`provider` field. When absent, `DEFAULT_PROVIDER = "local"`.

Adapters MUST register only `sandbox::provider::<name>::*` ids. They MUST NOT
shadow the bare `sandbox::*` namespace owned by the router.
