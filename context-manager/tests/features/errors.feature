@pure
Feature: error contract — spec strings, stable codes, tolerant unknowns

  Contract (context-manager.md § context::assemble Errors, plus the
  repo error convention `code: message`): `messages is required` and
  `could not resolve model limits` are the spec's literal strings;
  malformed shapes are rejected by deserialisation; unknown EXTRA
  fields are tolerated so older callers survive additive evolution.

  # Prevents: the documented error string drifting — callers match on
  # "messages is required" across all four functions.
  Scenario: assemble without messages fails with the spec string
    When I call "context::assemble" with:
      """
      { "model": { "id": "m1" } }
      """
    Then the call fails with code "context/invalid_request"
    And the error contains "messages is required"

  Scenario: compact without messages fails with the spec string
    When I call "context::compact" with:
      """
      { "model": { "id": "m1" } }
      """
    Then the call fails with code "context/invalid_request"
    And the error contains "messages is required"

  Scenario: prune without messages fails with the spec string
    When I call "context::prune" with:
      """
      {}
      """
    Then the call fails with code "context/invalid_request"
    And the error contains "messages is required"

  Scenario: count_tokens without messages fails with the spec string
    When I call "context::count_tokens" with:
      """
      { "model": { "id": "m1" } }
      """
    Then the call fails with code "context/invalid_request"
    And the error contains "messages is required"

  # Prevents: `messages: null` sneaking past as an empty history.
  Scenario: null messages are as missing as absent messages
    When I call "context::assemble" with:
      """
      { "messages": null, "model": { "id": "m1" } }
      """
    Then the call fails with code "context/invalid_request"
    And the error contains "messages is required"

  # Prevents: malformed roles silently coercing — a typo'd role must be
  # rejected at the boundary, not guessed at.
  Scenario: an unknown message role is rejected at deserialisation
    When I call "context::count_tokens" with:
      """
      { "messages": [{ "role": "wizard", "content": [], "timestamp": 1 }],
        "model": { "id": "m1" } }
      """
    Then the error contains "invalid request"

  # Prevents: a missing model slipping through and counting against a
  # phantom tokenizer.
  Scenario: count_tokens requires a model
    When I call "context::count_tokens" with:
      """
      { "messages": [] }
      """
    Then the error contains "invalid request"
    And the error contains "model"

  # Prevents: additive API evolution breaking old workers — unknown
  # extra request fields must be ignored, not fatal.
  Scenario: unknown extra fields are tolerated
    Given the router knows model "big" with context window 200000 and max output 8000
    When I call "context::assemble" with:
      """
      { "messages": [], "model": { "id": "big" }, "future_option": 42 }
      """
    Then the call succeeds
    And the response field "model_resolved" is "router"

  # Prevents: prune requiring a model it does not need — the heuristic
  # is model-free and the spec marks `model` optional here.
  Scenario: prune works without any model
    Given a user message "hello"
    When I call "context::prune" with:
      """
      { "messages": ${messages} }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
