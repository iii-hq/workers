@pure
Feature: model resolution and the usable token budget

  Contract (context-manager.md § Model input, § Token budget model):
  limits resolve inline -> router::models::get -> conservative fallback
  (8192/1024), echoed in `model_resolved`. The usable budget is
    usable = max(0, (input_limit ?? context_window - max_output_tokens)
                    - reserved - thinking_budget)
  with reserved defaulting to min(20000, 10% of context_window).
  assemble is the probe here because it returns both `usable` and
  `model_resolved`.

  Background:
    Given an empty history

  # Prevents: the worker phoning the router when the caller supplied
  # limits — inline limits are the standalone mode; a router lookup
  # would break deployments that don't install llm-router.
  Scenario: inline limits resolve without touching the router
    Given inline model "standalone" with context window 10000 and max output 1000
    When I assemble the history with model "standalone"
    Then the call succeeds
    And the response field "model_resolved" is "inline"
    And the response field "usable" is 8000
    And the router was never queried

  # Prevents: ignoring a declared input_limit and overfilling models
  # whose usable input is smaller than context - output.
  Scenario: input_limit takes precedence over the derived base
    Given inline model "capped" with input limit 5000, context window 10000 and max output 1000
    When I assemble the history with model "capped"
    Then the response field "usable" is 4000

  # Prevents: the reserve being un-overridable — latency-sensitive
  # callers tune it per call.
  Scenario: reserved_tokens overrides the default reserve
    Given inline model "standalone" with context window 10000 and max output 1000
    When I assemble the history with model "standalone" and options:
      """
      { "reserved_tokens": 0 }
      """
    Then the response field "usable" is 9000

  # Prevents: a flat 20k reserve eating a third of a small model's
  # window — the reserve must scale down with the context window.
  Scenario: the default reserve is 10% capped at 20000
    Given the router knows model "small" with context window 32000 and max output 16000
    When I assemble the history with model "small"
    Then the response field "model_resolved" is "router"
    And the response field "usable" is 12800
    And the router was asked for model "small"

  # Prevents: silent fallback — a caller must be able to detect that
  # conservative limits were used instead of real catalog data.
  Scenario: an unknown model falls back conservatively and says so
    When I assemble the history with model "ghost-model"
    Then the call succeeds
    And the response field "model_resolved" is "fallback"
    And the response field "usable" is 6349

  # Prevents: treating a dead router differently from a missing model —
  # both degrade to the same detectable fallback.
  Scenario: a dead router also falls back conservatively
    Given the router is unavailable
    When I assemble the history with model "any-model"
    Then the call succeeds
    And the response field "model_resolved" is "fallback"

  # Prevents: deployments that disabled the fallback silently running
  # on 8k limits anyway — they asked for a hard error, give them one.
  Scenario: fallback disabled turns an unresolvable model into an error
    Given config allow_fallback_limits is false
    When I assemble the history with model "ghost-model"
    Then the call fails with code "context/model_unresolved"
    And the error contains "could not resolve model limits"

  # Prevents: thinking tokens overflowing the context — when a tier is
  # requested and the model declares a budget, that budget must be
  # carved out of usable.
  Scenario: a requested thinking tier reserves its declared budget
    Given the router knows model "thinker" with context window 100000 and max output 10000
    And the router model "thinker" declares a thinking budget of 16000 for "high"
    When I assemble the history with model "thinker" and options:
      """
      { "thinking_level": "high" }
      """
    Then the response field "usable" is 64000

  # Prevents: reserving thinking room the model never declared — an
  # undeclared tier must cost nothing.
  Scenario: an undeclared thinking tier reserves nothing
    Given the router knows model "thinker" with context window 100000 and max output 10000
    And the router model "thinker" declares a thinking budget of 16000 for "high"
    When I assemble the history with model "thinker" and options:
      """
      { "thinking_level": "low" }
      """
    Then the response field "usable" is 80000

  # Prevents: inline-limit callers paying a thinking tax — inline
  # limits carry no budgets, so the tier must be a no-op.
  Scenario: inline limits ignore the thinking level entirely
    Given inline model "standalone" with context window 10000 and max output 1000
    When I assemble the history with model "standalone" and options:
      """
      { "thinking_level": "xhigh" }
      """
    Then the response field "usable" is 8000

  # Prevents: integer underflow wrapping a tiny budget around to ~2^64
  # — the budget must clamp to 0, never wrap.
  Scenario: a model smaller than its own output budget clamps to zero
    Given inline model "tiny" with context window 1000 and max output 2000
    When I assemble the history with model "tiny"
    Then the call succeeds
    And the response field "usable" is 0
