# Changelog

## 1.13.0

### Added

- **Autocompact** (chat-projects phase 3). A conversation compacts by itself when its
  context passes a number of tokens set globally (`autocompact {mode: "off" | "tokens",
  tokens?, instructions?}` in the configuration, through `harness::chat-settings`), per
  project (`autocompact`, accepted by `harness::projects::create` too) or per conversation
  (`metadata.autocompact`), each `{mode: "inherit" | "tokens" | "off", tokens?}`. The first
  level that is not `inherit` decides; a side chat follows its conversation, a sub-agent its
  project. Writes are checked: `bad_setting`, `bad_tokens` (20,000-2,000,000) and
  `bad_instructions` (over 1,000 characters).
- **The end of a turn** compacts once the last step's billed input (`input + cache_read +
  cache_write`; the snapshot's `total` when the step reported none) plus its output reaches
  the threshold (when it is below the model's `usable`), before the queue drains: the record
  stays `running` with `compacting`, a send queues and runs after it, a Stop abandons the
  summary (`status: "cancelled"`) and only that (a message queued during it still runs),
  sub-agents never do it, and `harness::on-turn-completed` carries the refreshed snapshot.
  `harness::side-chat::open` takes a compacting turn's reply as final;
  `harness::side-chat::summarize` answers `side_chat_busy` while it compacts.
- **Inside a turn** `context::assemble` gets `compaction_budget` (the setting plus
  `max(10 %, 20k)`), `compaction_instructions` and the turn's `provider_options`
  (context-manager 1.3.0). A failed or busy inline attempt drops the budget for the rest of
  the turn; a context-manager without the budget echo runs the turn without it.
- **Contract C on every compaction**: the `compaction` record and `harness::on-compaction`
  carry `trigger` (`command`, `auto`, `inline`, `limit`), `source`, `threshold_tokens`,
  `rule`, `context_before` and `context_after`; inline compactions get a `compaction_id` and
  emit their `completed` phase. The context snapshot gains `autocompact {setting_tokens,
  effective_tokens, cap_tokens, rule}`.
- **The notices say why** a compaction failed, from context-manager's `failure`: "a larger
  context window" only for an input too large, and a usage limit names the account with no
  clock time. After a failure, or a gain under 10 %, the next attempt waits until the
  context grew by another 10 % of the threshold (`harness_autocompact/<session_id>`); a usage
  limit starts no back-off.
- **`harness::autocompact::models`** (the models that ran a turn here in the last 7 days,
  with their limits) and **`harness::autocompact::log`** (the last 50 automatic
  compactions), both internal and denied to agents. Deleting a session drops its back-off
  and its log entries.

### Changed

- `/compact` and `harness::compact` run the summary on the conversation's Claude Code
  account (resolved at the call; a side chat uses its lock's) instead of the global one.

### Rollback

The configuration refuses unknown fields. Once the global autocompact setting holds
anything but its default (a mode of `tokens`, or a size or instructions, which stay stored
while the mode is `off`), a harness below 1.13 does not boot on that configuration: remove
`autocompact` from the `harness` configuration entry before rolling back. The default (off,
with no size and no instructions) is never stored, so an install that never set a size, or
cleared it, keeps a configuration older versions read. The other state it writes
(`harness_autocompact`, `harness_autocompact_log`, `harness_autocompact_models`, and the
new fields on turn records, snapshots and compaction records) is ignored by older versions.
