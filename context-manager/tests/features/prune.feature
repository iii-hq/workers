@pure
Feature: context::prune — placeholder verbose function outputs

  Contract (context-manager.md § context::prune, § Structural
  invariants): walk function_result content newest to oldest, freeing
  outputs outside a protected token window. Prune REPLACES, never
  removes — the block, the message, and the function_call_id linkage
  all survive; placeholders carry the freed size
  (`[output pruned: was ~N tokens]`). The most recent two user turns
  are always exempt (prior-art guard, independent of the window).

  Background:
    Given a user message "investigate the failure"
    And an assistant function call "c1" to "shell::run"
    And a function result for call "c1" from "shell::run" of ~2000 tokens
    And an assistant message "found it"
    And a user message "now fix it"
    And a user message "and add a test"

  # Prevents: deleting messages to save tokens — providers reject a
  # function_call whose result vanished, so the message count and the
  # call linkage must survive pruning.
  Scenario: pruning replaces content but never removes a message
    When I prune the history with options:
      """
      { "protect_recent_tokens": 100, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 1
    And the response field "pruned_tokens" is 2000
    And the response messages have as many messages as the request
    And every response message keeps its function_call_id
    And call/result pairing is intact in the response messages

  # Prevents: vague placeholders — the spec's placeholder must tell the
  # model (and a debugging human) how much was cut.
  Scenario: the placeholder names the freed size
    When I prune the history with options:
      """
      { "protect_recent_tokens": 100, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    Then response message 2 text is "[output pruned: was ~2000 tokens]"

  # Prevents: destroying context for a marginal win — freeing less than
  # min_free_tokens must leave the history completely untouched.
  Scenario: a pass that would free too little is a complete no-op
    When I prune the history with options:
      """
      { "protect_recent_tokens": 100, "min_free_tokens": 5000, "max_output_chars": 100 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
    And the response field "pruned_tokens" is 0
    And the response field "scanned_parts" is 1
    And response message 2 text has 8000 chars

  # Prevents: pruning the outputs a deployment declared load-bearing
  # (e.g. a plan document the model must keep verbatim).
  Scenario: protected functions are never pruned
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 100,
        "protected_functions": ["shell::run"] }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
    And the response field "scanned_parts" is 0
    And response message 2 text has 8000 chars

  # Prevents: the protected window filling oldest-first — the NEWEST
  # outputs are the ones the model still needs, so they must consume
  # the window and push older outputs out of it.
  Scenario: the newest outputs fill the protected window first
    Given an assistant function call "c2" to "coder::read"
    And a function result for call "c2" from "coder::read" of ~90 tokens
    And a user message "thanks"
    And a user message "continue"
    When I prune the history with options:
      """
      { "protect_recent_tokens": 100, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 1
    And response message 7 text has 360 chars
    And response message 2 text is "[output pruned: was ~2000 tokens]"

  # Prevents: pruning cheap outputs for cosmetic gains — an output at
  # or under max_output_chars is not "verbose" and stays, even outside
  # the protected window.
  Scenario: small outputs are not verbose and survive
    Given an empty history
    And a user message "start"
    And an assistant function call "c9" to "config::get"
    And a function result for call "c9" from "config::get" saying "value=42"
    And a user message "ok"
    And a user message "done"
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
    And the response field "scanned_parts" is 1
    And response message 2 text is "value=42"

  # Prevents: the always-exempt recent turns being pruned under token
  # pressure — outputs the user is actively working with must survive
  # even a zero-token protected window.
  Scenario: outputs inside the last two user turns are untouchable
    Given an empty history
    And a user message "look at this"
    And a user message "do the thing"
    And an assistant function call "c5" to "shell::run"
    And a function result for call "c5" from "shell::run" of ~3000 tokens
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
    And the response field "scanned_parts" is 0
    And response message 3 text has 12000 chars

  # Prevents: re-running prune from double-pruning placeholders or
  # re-counting freed tokens — the pass must be idempotent.
  Scenario: pruning twice is idempotent
    When I prune the history with options:
      """
      { "protect_recent_tokens": 100, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    And I prune the response messages again with the same options:
      """
      { "protect_recent_tokens": 100, "min_free_tokens": 1, "max_output_chars": 100 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
    And the response field "pruned_tokens" is 0
