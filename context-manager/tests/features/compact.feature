@pure
Feature: context::compact — summarise the head, keep a verbatim tail

  Contract (context-manager.md § context::compact): summarise the head
  of a history into a single compaction summary, keeping a recent tail
  verbatim. Transient and storage-agnostic — the caller persists the
  result; no session is touched. The response is a status union:
  ok | busy | empty | overflow. `tail_start_index` indexes the request
  messages array (this worker never sees storage entry ids).

  # Prevents: summarising nothing and burning an LLM call on an empty
  # transcript.
  Scenario: an empty history is empty, not an error
    Given an empty history
    And the router knows model "big" with context window 200000 and max output 8000
    When I compact the history with model "big"
    Then the call succeeds
    And the response field "status" is "empty"
    And the summariser was never invoked
    And no lease claim remains

  # Prevents: the core contract breaking — the head must be summarised,
  # the tail index must point into the REQUEST array, and the lease
  # must be released for the next caller.
  Scenario: a long head is summarised and the recent tail survives verbatim
    Given the router knows model "big" with context window 200000 and max output 8000
    And the summariser returns "## Goal\n- finish the migration"
    And a user message of ~9000 tokens
    And an assistant message "r1"
    And a user message "q2"
    And an assistant message "r2"
    And a user message "q3"
    And an assistant message "r3"
    When I compact the history with model "big"
    Then the call succeeds
    And the response field "status" is "ok"
    And the response field "summary" is "## Goal\n- finish the migration"
    And the response field "tail_start_index" is 2
    And the response field "tokens_before" exceeds 9000
    And the response field "tokens_after" does not exceed 1000
    And the response field "used_prior_summary" is false
    And the summariser was invoked 1 time
    And no lease claim remains

  # Prevents: tail_turns being ignored — operators tune how much recent
  # conversation survives compaction verbatim.
  Scenario: tail_turns controls how many recent turns survive
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message of ~9000 tokens
    And an assistant message "r1"
    And a user message "q2"
    And an assistant message "r2"
    And a user message "q3"
    And an assistant message "r3"
    When I compact the history with model "big" and options:
      """
      { "tail_turns": 1 }
      """
    Then the response field "status" is "ok"
    And the response field "tail_start_index" is 4

  # Prevents: callers misreading "no tail" — when everything was
  # summarised the index must be an explicit null, not 0 or missing.
  Scenario: tail_turns zero summarises everything and reports a null tail
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message "only message"
    And an assistant message "only reply"
    When I compact the history with model "big" and options:
      """
      { "tail_turns": 0 }
      """
    Then the response field "status" is "ok"
    And the response field "tail_start_index" is null

  # Prevents: convergence breaking — with a prior summary the
  # summariser must be anchored on it (update-in-place), not start
  # over, and the response must say the anchor was used.
  Scenario: a previous summary anchors the summariser
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big" and options:
      """
      { "previous_summary": "## Goal\n- the prior anchor" }
      """
    Then the response field "status" is "ok"
    And the response field "used_prior_summary" is true
    And the summariser system prompt contains "Update the anchored summary"
    And the summariser system prompt contains "the prior anchor"

  # Prevents: a first compaction accidentally running in update mode
  # against a phantom summary.
  Scenario: a first compaction creates a fresh summary
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And the summariser system prompt contains "Create a new anchored summary"
    And the summariser system prompt does not contain "<previous-summary>"

  # Prevents: "no router" looking like success — the spec maps a
  # missing summariser to overflow so callers treat compaction as
  # unavailable, and the lease must not stay stuck.
  Scenario: an unavailable summariser reports overflow and frees the lease
    Given the router knows model "big" with context window 200000 and max output 8000
    And the summariser is unavailable
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the call succeeds
    And the response field "status" is "overflow"
    And no lease claim remains

  # Prevents: provider failures mid-summarisation leaking as errors or
  # stuck leases.
  Scenario: a failing summariser reports overflow and frees the lease
    Given the router knows model "big" with context window 200000 and max output 8000
    And the summariser fails with "429 rate limited"
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the response field "status" is "overflow"
    And no lease claim remains

  # Prevents: persisting an empty string as the conversation's memory.
  Scenario: an empty summary is reported as empty, never persisted as ok
    Given the router knows model "big" with context window 200000 and max output 8000
    And the summariser returns an empty summary
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the response field "status" is "empty"
    And no lease claim remains

  # Prevents: base64 image payloads being shipped to the summariser —
  # they cost real tokens and carry nothing summarisable.
  Scenario: images are stripped before the summariser sees the head
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message containing an image of 400 chars
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And the summariser user prompt contains "[image stripped]"
    And the summariser user prompt does not contain "AAAA"

  # Prevents: a single huge tool output dominating the summariser's
  # input — function outputs are truncated to max_output_chars first.
  Scenario: verbose function outputs are truncated before summarisation
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message "start"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~5000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And the summariser user prompt contains "... [truncated]"

  # Prevents: the summariser running on a surprise model — it must run
  # on exactly the model the caller passed.
  Scenario: the summariser runs on the request model
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And the summariser ran on model "big"
