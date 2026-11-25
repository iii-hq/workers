# Changelog

## 0.3.0

Turns run on their own Claude Code account, and an account's usage limit is
told apart from a busy moment. Needs llm-router 1.6.0. Earlier versions were
published from `main` without a changelog.

### Added

- **Accounts from the tmux worker.** When the tmux worker registers
  `tmux::accounts::credential` (found with `engine::functions::list
  {include_internal: true}`, kept 30 s), every token comes from it: a stream
  uses `provider_options.account`, the account the harness froze on the turn;
  a stream without one, model discovery and token counting use the global
  account, named by `tmux::accounts::list`'s `global` id. Tokens are cached per
  account until a minute before they expire, and an answer that arrives after
  the account's token was dropped is never cached. tmux's answer must name the
  account asked for. The source sits behind an
  `AccountSource` trait (`src/accounts.rs`), so the accounts can move into this
  provider later without touching its callers.
- **A 401 asks once more.** On a 401 with an account's token, the provider drops
  that token, asks tmux again and sends the request once more with a different
  token; a 401 that survives is `auth_expired` and names the account, while
  tmux giving no token on that second ask keeps its own kind (`busy` stays
  `transient`). Streaming, discovery and token counting all do this.
- **Usage limits.** A 429 whose `anthropic-ratelimit-unified-status` is
  `rejected` is the account's usage limit: the error frame stays `rate_limited`
  and carries `error_details: {usage_limit: true, window?, resets_at_ms?}`
  (`five_hour` or `week`, from the representative claim; the reset in
  milliseconds), which the router never retries. Any other 429 is the plain
  `rate_limited` it was.

### Changed

- **No vault while tmux holds the accounts.** The provider then neither reads
  nor writes the `auth-credentials` vault, and skips its boot import of
  `~/.claude/.credentials.json`. tmux's refusals become the turn's error:
  `not_signed_in` and `token_expired` are `auth_expired`, `busy` is
  `transient`, `unknown_account` is `permanent`.
- **A pinned account never falls back.** A turn pinned to an account runs on
  its token or fails as `transient`: without the tmux worker, or while its
  function list cannot be read (tried three times, never taken for absence),
  it never runs on the `~/.claude` token of another account.
- Without the tmux worker, calls on the global account get their credentials
  from the vault or `~/.claude` as before. The boot import of `~/.claude` runs
  only when the tmux worker's absence is confirmed.
- Tokens never appear in `Debug` output (`AccountToken`, `Credential`,
  `ClaudeCodeConfig`, a failed count).
