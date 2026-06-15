@pure
Feature: context::assemble — the model-ready context pipeline

  Contract (context-manager.md § context::assemble): count -> (if over)
  prune function outputs -> (if still over) compact the head ->
  assemble the final list. The response reports what actually happened
  (`applied`), the budget it fit into (`usable`), and how the model was
  resolved. Degradations (busy lease, failed summariser, disabled
  steps) are best effort and visible — never an error on the hot path.

  # Prevents: the happy path being mangled — a context under budget
  # must pass through byte-identical, with nothing applied and no
  # summariser cost.
  Scenario: under budget passes through untouched
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message "hello"
    And an assistant message "hi there"
    When I assemble the history with model "big" and system prompt "You are helpful."
    Then the call succeeds
    And the response field "system_prompt" is "You are helpful."
    And the response field "token_count" does not exceed 100
    And the response field "applied.pruned" is false
    And the response field "applied.compacted" is false
    And the response field "applied.pruned_tokens" is 0
    And the response has no field "applied.summary"
    And the response messages equal the request history
    And the summariser was never invoked

  # Prevents: prune kicking in while the context still fits — pruning
  # under budget destroys context for nothing.
  Scenario: prune never runs under budget
    Given the router knows model "big" with context window 200000 and max output 8000
    And config "protect_recent_tokens" is 0
    And config "min_free_tokens" is 1
    And a user message "task"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~5000 tokens
    And a user message "next"
    And a user message "done"
    When I assemble the history with model "big"
    Then the response field "applied.pruned" is false
    And response message 2 text has 20000 chars

  # Prevents: an over-budget context reaching the model when freeing
  # old tool outputs would have been enough — the cheap pass must run
  # first and suffice alone.
  Scenario: over budget, prune alone brings the context home
    Given inline model "small" with context window 5000 and max output 500
    And config "protect_recent_tokens" is 0
    And config "min_free_tokens" is 1
    And the summariser returns "should never be needed"
    And a user message "task"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~5000 tokens
    And an assistant message "ok"
    And a user message "next"
    And a user message "done"
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.pruned" is true
    And the response field "applied.pruned_tokens" is 4992
    And the response field "applied.compacted" is false
    And the response field "token_count" does not exceed 4000
    And response message 2 text is "[output pruned: was ~5000 tokens]"
    And the response messages have as many messages as the request
    And call/result pairing is intact in the response messages
    And the summariser was never invoked

  # Prevents: prune running against the caller's explicit wishes — and
  # proves the over-budget result is still returned, visibly over.
  Scenario: disabling prune and compaction returns a visible best effort
    Given inline model "small" with context window 5000 and max output 500
    And config "protect_recent_tokens" is 0
    And config "min_free_tokens" is 1
    And a user message "task"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~5000 tokens
    And an assistant message "ok"
    And a user message "next"
    And a user message "done"
    When I assemble the history with model "small" and options:
      """
      { "allow_prune": false, "allow_compaction": false }
      """
    Then the call succeeds
    And the response field "applied.pruned" is false
    And the response field "applied.compacted" is false
    And the response field "token_count" exceeds 4000
    And response message 2 text has 20000 chars

  # Prevents: protected functions being pruned on the assemble path —
  # the per-call protection list must reach the prune pass.
  Scenario: protected functions survive the assemble prune pass
    Given inline model "small" with context window 5000 and max output 500
    And config "protect_recent_tokens" is 0
    And config "min_free_tokens" is 1
    And a user message "task"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~5000 tokens
    And an assistant message "ok"
    And a user message "next"
    And a user message "done"
    When I assemble the history with model "small" and options:
      """
      { "protected_functions": ["shell::run"], "allow_compaction": false }
      """
    Then the response field "applied.pruned" is false
    And response message 2 text has 20000 chars
    And the response field "token_count" exceeds 4000

  # Prevents: histories with nothing prunable never compacting — when
  # prune cannot help, compaction must take over and the returned tail
  # must map onto the REQUEST message indices.
  Scenario: still over after prune, the head is compacted away
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "## Goal\n- ship the feature"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small" and system prompt "Base prompt."
    Then the call succeeds
    And the response field "applied.compacted" is true
    And the response field "applied.summary" is "## Goal\n- ship the feature"
    And the response field "applied.tail_start_index" is 3
    And the response field "applied.tokens_before" exceeds 6000
    And the response field "token_count" does not exceed 4000
    And the response field "system_prompt" contains "Base prompt."
    And the response field "system_prompt" contains "# Conversation summary"
    And the response field "system_prompt" contains "ship the feature"
    And the response messages start at request message 3
    And the summariser was invoked 1 time
    And no lease claim remains

  # Prevents: both passes fighting instead of stacking — prune frees
  # the tool output, compaction then folds the rest of the head.
  Scenario: prune and compaction stack when one is not enough
    Given inline model "small" with context window 5000 and max output 500
    And config "protect_recent_tokens" is 0
    And config "min_free_tokens" is 1
    And the summariser returns "## Goal\n- both passes ran"
    And a user message "task"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~3000 tokens
    And an assistant message "ok"
    And a user message of ~4000 tokens
    And an assistant message "r1"
    And a user message "recent"
    And an assistant message "fine"
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.pruned" is true
    And the response field "applied.pruned_tokens" is 2992
    And the response field "applied.compacted" is true
    And the response field "applied.tail_start_index" is 5
    And the response field "token_count" does not exceed 4000
    And the response messages start at request message 5
    And call/result pairing is intact in the response messages

  # Prevents: compaction running against the caller's explicit wishes.
  Scenario: allow_compaction false leaves an over-budget context as is
    Given inline model "small" with context window 5000 and max output 500
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small" and options:
      """
      { "allow_compaction": false }
      """
    Then the call succeeds
    And the response field "applied.compacted" is false
    And the response field "token_count" exceeds 4000
    And the summariser was never invoked

  # Prevents: a busy lease failing the turn — assemble must degrade to
  # best effort, spend no summariser call, and leave the holder alone.
  Scenario: a busy lease degrades assemble to best effort
    Given inline model "small" with context window 5000 and max output 500
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    And a foreign lease on the history's default key taken 0 ms ago
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is false
    And the response field "token_count" exceeds 4000
    And the summariser was never invoked
    And the lease on the history's default key still belongs to the foreign holder

  # Prevents: a summariser failure erroring the hot path or leaving the
  # lease stuck — the turn proceeds over budget, the lease is freed.
  Scenario: a summariser failure degrades assemble to best effort
    Given inline model "small" with context window 5000 and max output 500
    And the summariser fails with "provider exploded"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is false
    And the response field "token_count" exceeds 4000
    And no lease claim remains
