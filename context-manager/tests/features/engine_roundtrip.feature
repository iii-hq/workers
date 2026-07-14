@engine
Feature: engine round trip — the production surface over a live bus

  The four functions registered in-process with PRODUCTION adapters
  (router resolver, router summariser, state-backed leases) against a
  real engine. llm-router is absent on the test engine — which is
  exactly the degraded mode the spec defines: limits fall back, prune
  and count stay fully functional, compact reports overflow after a
  real state::* lease acquire/release cycle. Scenarios soft-skip when
  no engine is reachable.

  Background:
    Given an engine connection

  # Prevents: the registered wire surface drifting from the handlers —
  # a real trigger must reach the production count path end to end.
  Scenario: count_tokens answers over the bus
    Given a user message "hello engine"
    When I call "context::count-tokens" on the engine with:
      """
      { "messages": ${messages}, "model": { "id": "any-model" } }
      """
    Then the call succeeds
    And the response field "estimator" is "heuristic"
    And the response field "tokens" exceeds 0

  # Prevents: prune needing anything beyond the bus — no router, no
  # state, just the transform.
  Scenario: prune rewrites outputs over the bus
    Given a user message "investigate"
    And an assistant function call "e1" to "shell::run"
    And a function result for call "e1" from "shell::run" of ~2000 tokens
    And a user message "next"
    And a user message "done"
    When I call "context::prune" on the engine with:
      """
      { "messages": ${messages},
        "options": { "protect_recent_tokens": 100, "min_free_tokens": 1, "max_output_chars": 100 } }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 1
    And response message 2 text is "[output pruned: was ~2000 tokens]"

  # Prevents: assemble requiring llm-router for the inline-limits path
  # — the standalone mode must work on a bare engine.
  Scenario: assemble passes through under budget with inline limits
    Given a user message "hello"
    When I call "context::assemble" on the engine with:
      """
      { "messages": ${messages},
        "model": { "id": "standalone",
                   "limits": { "context_window": 10000, "max_output_tokens": 1000 } },
        "system_prompt": "You are helpful." }
      """
    Then the call succeeds
    And the response field "model_resolved" is "inline"
    And the response field "usable" is 8000
    And the response field "applied.compacted" is false
    And the response field "system_prompt" is "You are helpful."

  # A missing summariser cannot weaken the hard assembly budget. The
  # real state::* lease cycle still has to release before overflow returns.
  Scenario: over budget without a router fails closed
    Given a user message of ~3000 tokens
    And a user message of ~3000 tokens
    And a user message "recent"
    When I call "context::assemble" on the engine with:
      """
      { "messages": ${messages},
        "model": { "id": "small",
                   "limits": { "context_window": 5000, "max_output_tokens": 500 } } }
      """
    Then the call fails with code "context/overflow"

  # Prevents: "router absent" surfacing as anything but the documented
  # overflow status — and proves the real lease cycle completes (a
  # second compact is NOT busy).
  Scenario: compact without a router reports overflow and frees its lease
    Given a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"
    When I call "context::compact" on the engine with:
      """
      { "messages": ${messages},
        "model": { "id": "big",
                   "limits": { "context_window": 200000, "max_output_tokens": 8000 } } }
      """
    Then the call succeeds
    And the response field "status" is "overflow"
    When I call "context::compact" on the engine with:
      """
      { "messages": ${messages},
        "model": { "id": "big",
                   "limits": { "context_window": 200000, "max_output_tokens": 8000 } } }
      """
    Then the response field "status" is "overflow"

  # Prevents: handler errors losing their stable code across the bus.
  Scenario: the spec error string survives the wire
    When I call "context::assemble" on the engine with:
      """
      { "model": { "id": "m1" } }
      """
    Then the error contains "context/invalid_request"
    And the error contains "messages is required"
