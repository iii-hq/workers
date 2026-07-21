@pure
Feature: context::assemble — the model-ready context pipeline

  Contract (context-manager.md § context::assemble): count -> (if over)
  prune function outputs -> (if still over) compact the head ->
  assemble the final list. The response reports what actually happened
  (`applied`), the budget it fit into (`usable`), and how the model was
  resolved. Every successful response fits its reported usable budget.
  Busy leases, failed summarisers, disabled passes, and irreducible
  inputs fail with context/overflow instead of leaking an invalid request.

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

  # Regression for MOT-4014: the newest result is inside every normal
  # protection window, but a multi-megabyte result must still become a
  # bounded transcript reference before the request reaches a provider.
  Scenario: a 4.98 MB latest function result is reduced despite recent-turn protection
    Given inline model "large" with context window 272000 and max output 128000
    And config "protect_recent_tokens" is 2000000
    And config "min_free_tokens" is 2000000
    And a user message "inspect the session"
    And an assistant function call "latest-call" to "session::messages"
    And a function result for call "latest-call" from "session::messages" of ~1245000 tokens
    When I assemble the history with model "large"
    Then the call succeeds
    And the response field "applied.pruned" is true
    And the response field "token_count" does not exceed 124000
    And response message 2 text does not exceed 1000 chars
    And response message 2 text contains "session transcript"
    And the response field "messages.2.details.context_reference.kind" is "function_result_reference"
    And the response field "messages.2.details.context_reference.original_estimated_tokens" exceeds 1200000
    And the response field "messages.2.details.context_reference.retrieval_hint" contains "function_call_id"
    And the response messages have as many messages as the request
    And every response message keeps its function_call_id
    And call/result pairing is intact in the response messages

  # The optional normal passes may be disabled, but that does not disable
  # the terminal safety reduction required to uphold the hard budget.
  Scenario: disabling normal prune and compaction still cannot return over budget
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
    And the response field "applied.pruned" is true
    And the response field "applied.compacted" is false
    And the response field "token_count" does not exceed 4000
    And response message 2 text does not exceed 1000 chars
    And call/result pairing is intact in the response messages

  # Explicit protection applies to the normal quality-preserving pass,
  # but cannot authorize an invalid provider request.
  Scenario: a protected oversized result yields to emergency reduction
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
    Then the call succeeds
    And the response field "applied.pruned" is true
    And response message 2 text does not exceed 1000 chars
    And the response field "token_count" does not exceed 4000
    And call/result pairing is intact in the response messages

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

  # Prevents: an oversized final turn being summarised into an EMPTY
  # model context. When no recent turn fits the verbatim-tail budget,
  # compaction must keep the last turn rather than fold everything away —
  # the provider rejects an empty messages array.
  Scenario: an oversized final turn is kept verbatim, never emptied
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "## Goal\n- keep going"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is true
    And the response field "token_count" does not exceed 4000
    And the response messages are not empty
    And the response messages start at request message 2

  # Prevents: a candidate window with NO user turn (the harness opens
  # windows at a prior compaction's tail_start, which may be an assistant
  # boundary) being summarised into an EMPTY model context. select()
  # returns a whole-head selection for a user-less view; compaction must
  # skip it and leave the shrinking to emergency reduction. Observed live
  # 2026-07-21: turn failed "context::assemble returned an empty
  # model-facing context".
  Scenario: a user-less window is never compacted to empty
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "## Goal\n- never used"
    And an assistant function call "c1" to "coder::read-file"
    And a function result for call "c1" from "coder::read-file" of ~6000 tokens
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is false
    And the response messages are not empty
    And the response field "token_count" does not exceed 4000
    And the summariser was invoked 0 times
    And call/result pairing is intact in the response messages

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

  # Disabling compaction is respected, but an irreducible request fails
  # locally instead of being returned over budget.
  Scenario: allow_compaction false reports an irreducible overflow
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
    Then the call fails with code "context/overflow"
    And the summariser was never invoked

  # A busy lease cannot weaken the final budget postcondition.
  Scenario: a busy lease reports overflow for irreducible content
    Given inline model "small" with context window 5000 and max output 500
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    And a foreign lease on the history's default key taken 0 ms ago
    When I assemble the history with model "small"
    Then the call fails with code "context/overflow"
    And the summariser was never invoked
    And the lease on the history's default key still belongs to the foreign holder

  # A summariser failure releases its lease and fails safely when no
  # remaining deterministic transform can fit the request.
  Scenario: a summariser failure reports overflow and releases the lease
    Given inline model "small" with context window 5000 and max output 500
    And the summariser fails with "provider exploded"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small"
    Then the call fails with code "context/overflow"
    And no lease claim remains

  # Invocation schemas are part of the provider input and must participate
  # in the same hard postcondition as messages and the system prompt.
  Scenario: tool schema overhead can make an otherwise small request overflow
    Given inline model "small" with context window 5000 and max output 500
    And a user message "hello"
    When I assemble the history with model "small" and request fields:
      """
      {
        "tools": [{
          "name": "large_tool",
          "description": "A deliberately verbose invocation contract used to prove that tool schemas are counted during assembly. This description repeats enough contract material to exceed a narrow remaining input budget while the user message itself still fits. The implementation must never omit this schema from the estimate because the exact same schema is serialized into the provider request after context assembly.",
          "parameters": {
            "type": "object",
            "properties": {
              "query": { "type": "string", "description": "A long search query with detailed matching and filtering semantics." },
              "scope": { "type": "string", "description": "The namespace and tenancy scope used to constrain the operation." },
              "cursor": { "type": "string", "description": "An opaque continuation cursor for bounded retrieval." }
            }
          },
          "execution_mode": "sequential"
        }],
        "options": { "reserved_tokens": 4350, "allow_compaction": false }
      }
      """
    Then the call fails with code "context/overflow"

  # Response-format and provider-specific fields are serialized outside the
  # message/tool arrays, so callers supply their measured overhead explicitly.
  Scenario: caller-supplied complete request overhead participates in the budget
    Given inline model "small" with context window 5000 and max output 500
    And a user message "hello"
    When I assemble the history with model "small" and request fields:
      """
      {
        "options": {
          "reserved_tokens": 4300,
          "request_overhead_tokens": 500,
          "allow_compaction": false
        }
      }
      """
    Then the call fails with code "context/overflow"

  # The advertised output allowance is deducted before input assembly. A
  # request that would fit only by stealing that allowance must fail locally.
  Scenario: model output reservation cannot be consumed by input context
    Given inline model "output-heavy" with context window 5000 and max output 4400
    And a user message of ~300 tokens
    When I assemble the history with model "output-heavy" and options:
      """
      { "allow_compaction": false }
      """
    Then the call fails with code "context/overflow"
