# Agent Guild

Two optional evidence functions help an iii caller examine a newly selected public
agent endpoint or verify a supplied public passport before choosing what to do
next. Endpoint observations preserve all six measured statuses and explicit
unknowns; passport verification checks separately expected issuer and subject DIDs.
Neither operation authorizes delegation, establishes endpoint ownership, validates
worker binaries, or makes a payment.

## Install

After this contribution is accepted and released to the registry:

```bash
iii trigger compose::add worker=agent-guild
```

Availability of the draft's source does not imply that registry installation is
already available. The worker requires Node 22+ and uses the two free Guild
operations without a Guild account, key or registration.

## Quickstart

With the worker running, observe an exact public endpoint selected for your task:

```bash
iii trigger agent-guild::preflight --json '{"url":"https://agent-guild-5d5r.onrender.com/mcp"}'
```

This example observes Guild's own public MCP endpoint. An `observed` result contains
the exact target, request/completion times, fixed service origin, HTTP status,
response size, all six check statuses, failed/unknown lists and a consistent verdict.
Unknown never becomes proven; `no_failed_checks` can still contain unknown checks.
No later endpoint invocation is bound to this observation.

To verify an already supplied **public** passport with a connected iii client:

```javascript
const result = await iii.trigger({
  function_id: 'agent-guild::verify_passport',
  payload: {
    credential: suppliedPublicPassport,
    expectedIssuerDid: independentlyExpectedIssuerDid,
    expectedSubjectDid: independentlyExpectedSubjectDid,
  },
});
```

Keep the expected DIDs independent of the credential: copying its claims into those
fields does not establish the intended counterparty. A completed verification
reports both remote verifier flags, signed dates and local time/binding results.
`verified` requires both flags plus validity and freshness checks; negative flags
remain a completed negative result. This is online verification, not independent
local cryptographic verification. A signature proves origin/integrity, not truthful
claims, safety, task quality or independent ownership.

**Disclosure:** only the exact endpoint string or complete supplied public
credential is sent to `https://agent-guild-5d5r.onrender.com`. Preflight actively
probes that endpoint. `POST /credentials/verify` records a `passport_verified` event,
including unsuccessful verification. Exclude secret URLs, confidential claims,
credentials and unrelated conversation content. Guild can log these requests;
the host engine may also retain invocation data under its own logging policies.

## Configuration

The packaged worker uses fixed defaults and no configuration file or provider
credentials. The iii SDK resolves its engine from `III_URL`, falling back to
`ws://127.0.0.1:49134`; `--url` selects an explicit engine. Connection settings are
host-owned and are not part of public registry defaults.

The exported `startWorker(address, options)` and `createOperations(options)` APIs
allow an embedding host to set `timeoutMs` (100–45000, default 15000),
`maxPassportAgeSeconds` (1–604800, default 86400), optional exact lowercase
`allowedHosts` and an optional additional `expectedIssuerDid` restriction. No host
list is required for a newly selected endpoint; an empty list disables preflight.
Every passport call still requires both expected DIDs. The transport origin and
routes are fixed.

Public HTTP(S) DNS URLs are supported; IP literals, local/reserved suffixes,
userinfo, fragments and malformed URLs are rejected. Lexical screening does not
prove DNS resolves publicly. The caller must supply public data; Guild separately
screens the endpoint. Credentials must be complete bounded JSON objects, at most
32 KiB and 16 nesting levels. Supported Ed25519 `did:key`, proof shape and signed
UTC validity/freshness are checked before and after the request, at millisecond
precision. Non-JSON values, accessors, cycles and unsafe keys are rejected.

Fetch requests disable redirects, credentials, cache and referrers. There are no
retries, paid fallbacks or target calls by this worker. The 64 KiB response cap
covers decoded stream bytes, not platform prefetch buffers or decompression work.
The deadline bounds waiting for fetch/body and sends abort; event-loop suspension
can delay timers. A host-controlled `fetch` override must honor `AbortSignal` to
stop underlying work: bounded waiting and abort do not physically terminate an
arbitrary override. Worker shutdown aborts outstanding work. An iii caller's
invocation timeout is separate and does not automatically cancel the HTTP work.

The exact top-level iii `_caller_worker_id` UUID field is consumed at registration
and ignored; it is not sent to Guild or used for authorization. Other unexpected
keys are rejected and nested public credential claims remain intact. Framework
channel-reference objects are not supported credential claims. Errors use fixed
local codes, and arbitrary remote prose is not forwarded in results.

Both functions explicitly advertise `metadata.mcp.expose: true`. Host MCP policy
still decides whether to expose them; this is discovery metadata, not a hook or
automatic execution. This worker disables its optional SDK metrics and OpenTelemetry;
engine and caller trace settings remain host-controlled. The SDK supports
`III_DISABLE_TRACE_PAYLOADS=1` for hosts that separately enable its tracing. No live
MCP bridge, deployed CORS, independent adoption or current-service claim is inferred
from the offline tests.
