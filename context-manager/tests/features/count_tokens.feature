@pure
Feature: context::count-tokens — estimate token usage for a message set

  Contract (context-manager.md § context::count-tokens): estimate token
  usage for messages, optionally including invocation schemas and a
  system prompt, vs a model. The model selects a tokenizer; v1 always
  falls back to the generic chars/4 heuristic and says so in
  `estimator`. Pure and router-free — safe for cost-sensitive callers
  with no llm-router installed.

  # Prevents: silent estimator swaps — callers budgeting real money on
  # these numbers must always be able to see which estimator ran.
  Scenario: every count declares the estimator that produced it
    Given a user message "hello"
    When I count tokens with model "any-model"
    Then the call succeeds
    And the response field "estimator" is "heuristic"
    And the router was never queried

  # Prevents: the total drifting away from the by_role breakdown —
  # callers subtract buckets (e.g. custom) from the total, so the two
  # views must reconcile exactly.
  Scenario: the total equals the sum of the by_role buckets
    Given a user message "what is the weather"
    And an assistant message "sunny with a chance of tokens"
    And a function result for call "c1" from "weather::get" saying "22 degrees"
    And a custom message of type "ui_marker"
    When I count tokens with model "any-model"
    Then the call succeeds
    And the response field "tokens" equals the sum of the "by_role" fields
    And the response field "by_role.user" exceeds 0
    And the response field "by_role.assistant" exceeds 0
    And the response field "by_role.function_result" exceeds 0
    And the response field "by_role.custom" exceeds 0

  # Prevents: count_tokens silently dropping custom messages the way
  # assemble does — this function counts what it is GIVEN; the by_role
  # split is how callers see (and can subtract) the custom share.
  Scenario: custom messages are counted here, unlike in assemble
    Given a custom message of type "compaction" of ~500 tokens
    When I count tokens with model "any-model"
    Then the call succeeds
    And the response field "by_role.custom" exceeds 499
    And the response field "tokens" exceeds 499

  # Prevents: the system prompt being forgotten in the estimate — it
  # occupies real context window and a 0-message count must reflect it.
  Scenario: the system prompt is part of the total but no role bucket
    Given an empty history
    When I count tokens with model "any-model" and system prompt "xxxxxxxxxxxxxxxx"
    Then the call succeeds
    And the response field "tokens" is 4
    And the response field "by_role.user" is 0
    And the response field "by_role.assistant" is 0

  # Prevents: invocation schemas (the agent_trigger entry) being
  # invisible to the budget — tool schemas can be thousands of tokens.
  Scenario: tools add to the total without polluting role buckets
    Given an empty history
    When I count tokens with model "any-model" and tools:
      """
      [{ "name": "agent_trigger", "description": "Invoke any allowed iii function.",
         "parameters": { "type": "object", "properties": { "function": { "type": "string" } } } }]
      """
    Then the call succeeds
    And the response field "tokens" exceeds 0
    And the response field "by_role.user" is 0
    And the response field "by_role.assistant" is 0
    And the response field "by_role.function_result" is 0
    And the response field "by_role.custom" is 0

  # Prevents: an empty history erroring or returning phantom tokens.
  Scenario: an empty message set counts to zero
    Given an empty history
    When I count tokens with model "any-model"
    Then the call succeeds
    And the response field "tokens" is 0
    And the response field "by_role.user" is 0

  # Prevents: named parts leaking into the total — parts are a side
  # breakdown for callers dissecting a prompt they also count whole
  # (e.g. system prompt segments), so double-billing must be impossible.
  Scenario: parts are counted individually and never added to the total
    Given an empty history
    When I count tokens with model "any-model" and parts:
      """
      { "identity": "xxxxxxxxxxxxxxxx", "guidance": "xxxxxxxx" }
      """
    Then the call succeeds
    And the response field "tokens" is 0
    And the response field "by_part.identity" is 4
    And the response field "by_part.guidance" is 2

  # Prevents: the tools share being indistinguishable inside the total —
  # budget UIs render schemas as their own category.
  Scenario: the tools share of the total is reported separately
    Given an empty history
    When I count tokens with model "any-model" and tools:
      """
      [{ "name": "agent_trigger", "description": "Invoke any allowed iii function.",
         "parameters": { "type": "object", "properties": { "function": { "type": "string" } } } }]
      """
    Then the call succeeds
    And the response field "tokens" exceeds 0
    And the response field "tools_tokens" equals the response field "tokens"
