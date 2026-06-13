@pure
Feature: structural invariants — the returned context is always provider-legal

  Contract (context-manager.md § Structural invariants): whatever
  pruning or compaction does, a function_call and its function_result
  always land on the same side of any boundary (providers reject
  orphaned results); prune replaces, never removes; custom messages
  never reach the model-facing list nor its token count.

  # Prevents: the compaction boundary slicing between an assistant's
  # call and its result — the only in-budget cut here straddles the
  # pair, so selection must skip it and cut after the result instead.
  Scenario: the tail never starts between a call and its result
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "## Goal\n- boundary test"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message "follow up"
    And an assistant function call "c2" to "shell::run"
    And a function result for call "c2" from "shell::run" of ~2100 tokens
    And an assistant message "done"
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is true
    And the response field "applied.tail_start_index" is 5
    And the response messages start at request message 5
    And call/result pairing is intact in the response messages
    And the summariser user prompt contains "[tool_call] shell::run"

  # Prevents: a turn that STARTS with an inline function_result block
  # (Anthropic-style tool results travel in user messages) being used
  # as a cut point — its call sits in the previous turn, so the cut
  # must move back to the older, safe turn and keep the pair together.
  Scenario: a turn starting with an inline result defers to an older safe cut
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "## Goal\n- inline result test"
    And a user message of ~4000 tokens
    And an assistant message "r0"
    And a user message "follow up"
    And an assistant function call "c7" to "coder::search"
    And a user message carrying an inline function result block for call "c7"
    And an assistant message "done"
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is true
    And the response field "applied.tail_start_index" is 2
    And the response messages start at request message 2
    And call/result pairing is intact in the response messages

  # Prevents: custom messages leaking to providers (they have no wire
  # mapping) or their tokens triggering phantom overflows — a huge
  # custom entry must neither appear nor count.
  Scenario: custom messages neither reach the model nor count tokens
    Given inline model "small" with context window 5000 and max output 500
    And a custom message of type "compaction" of ~10000 tokens
    And a user message "hi"
    When I assemble the history with model "small"
    Then the call succeeds
    And the response field "token_count" does not exceed 100
    And the response field "applied.pruned" is false
    And the response field "applied.compacted" is false
    And the response messages contain no custom roles
    And the response field "messages" has 1 items
    And the summariser was never invoked

  # Prevents: tail_start_index drifting when custom messages sit before
  # the cut — the index must point into the REQUEST array (customs
  # included), not the filtered view, or callers persist the wrong
  # boundary.
  Scenario: tail_start_index accounts for excluded custom messages
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "## Goal\n- index mapping"
    And a custom message of type "ui_marker"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small"
    Then the response field "applied.compacted" is true
    And the response field "applied.tail_start_index" is 4
    And the response messages contain no custom roles

  # Prevents: prune deleting blocks or messages under pressure — the
  # message count, the placeholder, and the call linkage must all
  # survive a maximally aggressive prune.
  Scenario: an aggressive prune still replaces rather than removes
    Given a user message "task"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~5000 tokens
    And an assistant function call "c2" to "coder::read"
    And a function result for call "c2" from "coder::read" of ~5000 tokens
    And a user message "next"
    And a user message "done"
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 0 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 2
    And the response messages have as many messages as the request
    And every response message keeps its function_call_id
    And call/result pairing is intact in the response messages
