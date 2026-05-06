# auth::list_providers

List providers that currently have a stored credential.

`() → { providers: [provider] }` — returns provider names only; tokens are
never returned by this function.

## When to use

- Building a settings UI that shows "you're connected to: Anthropic, OpenAI, ..."
- Auditing which providers a workspace has authenticated with.
- Pre-flight: detecting which providers are usable without trying each in turn.
