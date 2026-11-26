# Changelog

## 1.3.0

A caller can ask for compaction before the model's limit, on the
conversation's account, and learn why a summary failed. Earlier versions were
published from `main` without a changelog.

### Added

- **A soft compaction budget.** `context::assemble` takes
  `options.compaction_budget`: prune and compaction start once the request
  passes `min(usable, compaction_budget)`. Emergency reduction and the
  `context/overflow` error still hold to `usable` alone, so a busy or failed
  summariser at a low budget never drops content the model could still read,
  nor fails the call.
- **The echo.** The response's `compaction_budget` is the threshold the call
  applied (`min(usable, compaction_budget)`), or `null` without the option. It
  is always present, so a caller can tell an older context-manager, which
  drops the option silently, by the field's absence.
- **The compaction report.** When the call tried an inline compaction, the
  response carries `compaction: {trigger, status, failure?}`: `trigger` is
  `budget` (the request passed the budget while still within `usable`) or
  `usable` (the model's limit, as before); `status` is `compacted`, `failed`,
  `busy` (the lease is held) or `skipped` (nothing to summarise without taking
  the whole request).
- **Instructions for the inline compaction.**
  `options.compaction_instructions` steers the inline summary as
  `context::compact`'s `instructions` do: trimmed, ignored when blank, cut past
  2000 characters.
- **`provider_options`.** `context::assemble` and `context::compact` take
  `options.provider_options` (namespaced by provider id, such as the
  conversation's account under `claude-code`) and forward them to every
  `router::chat` call the summary makes. Without them the summary runs on the
  router's defaults, as before.
- **Why a summary failed.** `context::compact`'s `overflow` carries
  `failure: {kind, message, error_details?}`, and so does the assemble's
  `compaction.failure`. `kind` is `usage_limit`, `rate_limited`,
  `input_too_large`, `timeout`, `unavailable` or `other`; `input_too_large` is
  the only one a larger context window would fix. `error_details` is
  llm-router's (`{usage_limit, window?, resets_at_ms?}`), passed through, and
  `usage_limit: true` makes the kind `usage_limit`. A `context/overflow` error
  that follows a failed inline compaction ends with `; compaction failed: `
  and the failure as one JSON object; the message before it is unchanged.

### Changed

- **The provider's count starts earlier with a budget.** The estimate used to
  be checked against the provider's count only past a third of `usable`; with
  a budget it is checked from `min(usable / 3, 0.8 × compaction_budget)`. On a
  1M window a third is about 305k, so a 160k budget was decided on the
  estimate alone.
- **Summariser failures keep their cause.** A router reply, a stream `error`
  frame and a bus error are classified instead of all reading as one failure.
  A `router/context_overflow` reply is still retried with less history.

### Compatibility

- Only fields were added. An older harness reads `{status: "overflow"}` as
  before and ignores `failure`; the `context/overflow` code, and its message
  when no compaction failed, are unchanged.
- An older context-manager ignores the new options without an error. A caller
  that needs the budget checks for the `compaction_budget` echo.
