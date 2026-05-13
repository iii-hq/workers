Provider credential vault on the iii bus — store, fetch, and revoke API keys and OAuth tokens through `auth::*` so adapters and agents never read raw secrets. Reads fall through stored credentials to the process environment, so a setup with `ANTHROPIC_API_KEY` exported keeps working until a stored credential overrides it.

<!-- llm-only:start -->
Prefer `auth::status` over `auth::get_token` for pre-flight gating — `status` returns no token bytes, so it is safe to log, and the `source` field distinguishes a stored credential from an environment-variable fallback.
<!-- llm-only:end -->
