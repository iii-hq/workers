# Changelog

## 1.6.0

An account's usage limit is told apart from a busy moment. Earlier versions were
published from `main` without a changelog.

### Added

- **`error_details` on errors.** The assistant message (`AssistantMessage`, so
  the `error` frame and `router::complete`'s `message`) and the router's
  `ErrorShape` carry an optional `error_details: {usage_limit, window?,
  resets_at_ms?}`: `usage_limit` is true when the account's usage limit is
  reached, `window` is `five_hour` or `week` when the provider said which, and
  `resets_at_ms` is when that limit resets. `error_kind` stays `rate_limited`
  and `ErrorKind` keeps its five values. The field is written only when
  present, so a message without it reads and writes as before, and an older
  reader ignores it. A `window` this version does not know reads as absent,
  and unreadable details never drop the frame.

### Changed

- **A usage limit is never retried.** The router's retry policy skips a
  `rate_limited` terminal whose `error_details.usage_limit` is true: the
  provider is called once and the error reaches the caller with
  `retryable: false`, code `router/provider_rate_limited` and a message that
  says the account reached its usage limit. The details pass through on the
  response's `error`, on the streamed `error` frame and on the message.
  Other `rate_limited` errors keep their retries.
