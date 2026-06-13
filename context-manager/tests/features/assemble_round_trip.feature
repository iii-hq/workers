@pure
Feature: the compaction round trip — summaries persist, converge, never grow

  Contract (context-manager.md § The compaction round trip): the worker
  never persists a summary; the caller does. previous_summary is
  rendered into the system prompt under "# Conversation summary"; when
  compaction triggers again the summariser UPDATES that summary instead
  of starting over, so summaries converge. Callers that skip
  persistence still get correct output — at one summariser call per
  over-budget request.

  # Prevents: a persisted summary silently vanishing from the model's
  # view — the round trip's whole point is that the summary comes back
  # every turn.
  Scenario: a previous summary is rendered into the system prompt
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message "hello again"
    When I assemble the history with model "big", system prompt "Base." and options:
      """
      { "previous_summary": "## Goal\n- carried forward from last time" }
      """
    Then the call succeeds
    And the response field "applied.compacted" is false
    And the response field "system_prompt" contains "Base."
    And the response field "system_prompt" contains "# Conversation summary"
    And the response field "system_prompt" contains "carried forward from last time"
    And the summariser was never invoked

  # Prevents: stale and fresh summaries stacking up in the prompt — a
  # new compaction must REPLACE the rendered summary, not append to it.
  Scenario: a fresh compaction replaces the rendered previous summary
    Given inline model "small" with context window 5000 and max output 500
    And the summariser returns "NEW-SUMMARY-TEXT"
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small" and options:
      """
      { "previous_summary": "OLD-ANCHOR-TEXT" }
      """
    Then the response field "applied.compacted" is true
    And the response field "applied.summary" is "NEW-SUMMARY-TEXT"
    And the response field "system_prompt" contains "NEW-SUMMARY-TEXT"
    And the response field "system_prompt" does not contain "OLD-ANCHOR-TEXT"

  # Prevents: summaries growing forever — the second compaction must
  # run in update mode, anchored on the previous summary.
  Scenario: re-compaction is anchored on the previous summary
    Given inline model "small" with context window 5000 and max output 500
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small" and options:
      """
      { "previous_summary": "OLD-ANCHOR-TEXT" }
      """
    Then the response field "applied.compacted" is true
    And the summariser system prompt contains "Update the anchored summary"
    And the summariser system prompt contains "OLD-ANCHOR-TEXT"

  # Prevents: a first compaction hallucinating an anchor it never had.
  Scenario: a first compaction starts fresh
    Given inline model "small" with context window 5000 and max output 500
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small"
    Then the response field "applied.compacted" is true
    And the summariser system prompt contains "Create a new anchored summary"
    And the summariser system prompt does not contain "<previous-summary>"

  # Prevents: misunderstanding the cost model — a caller that never
  # persists the summary stays correct but pays one summariser call per
  # over-budget request. This scenario IS the documented trade-off.
  Scenario: skipping persistence costs one summariser call per request
    Given inline model "small" with context window 5000 and max output 500
    And a user message of ~3000 tokens
    And an assistant message "r0"
    And a user message of ~3000 tokens
    And an assistant message "r1"
    And a user message "recent question"
    And an assistant message "recent answer"
    When I assemble the history with model "small"
    And I assemble the history with model "small"
    Then the call succeeds
    And the response field "applied.compacted" is true
    And the summariser was invoked 2 times
