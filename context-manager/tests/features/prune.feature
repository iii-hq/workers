@pure
Feature: context::prune — placeholder eligible old function outputs

  Contract (context-manager.md § context::prune, § Structural
  invariants): walk function_result content newest to oldest, freeing
  outputs outside a protected token window. Prune REPLACES, never
  removes — the block, the message, and the function_call_id linkage
  all survive; placeholders name the source function and the freed
  size, and point back at the recovery path
  (`[output of {function_id} pruned: was ~N tokens; re-call it if
  still needed]`). A configurable number of recent user turns are exempt
  (default two, independent of the token window), and smaller outputs can
  become eligible after their configured decay age.

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
    And the response field "pruned_tokens" is 1982
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
    Then response message 2 text is "[output of shell::run pruned: was ~2000 tokens; re-call it if still needed]"

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
    And response message 2 text is "[output of shell::run pruned: was ~2000 tokens; re-call it if still needed]"

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

  Scenario: prune inherits configured decay controls
    Given config "decay_user_turns" is 2
    And config "protected_user_turns" is 0
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 10000 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 1
    And response message 2 text is "[output of shell::run pruned: was ~2000 tokens; re-call it if still needed]"

  Scenario: explicit zero disables configured decay
    Given config "decay_user_turns" is 1
    And config "protected_user_turns" is 0
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 10000,
        "decay_user_turns": 0, "protected_user_turns": 0 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 0
    And the response field "scanned_parts" is 1
    And response message 2 text has 8000 chars

  Scenario: explicit zero removes the configured recent-turn exemption
    Given an empty history
    And config "protected_user_turns" is 2
    And a user message "start"
    And an assistant function call "c9" to "shell::run"
    And a function result for call "c9" from "shell::run" of ~2000 tokens
    When I prune the history with options:
      """
      { "protect_recent_tokens": 0, "min_free_tokens": 1, "max_output_chars": 100,
        "decay_user_turns": 0, "protected_user_turns": 0 }
      """
    Then the call succeeds
    And the response field "pruned_parts" is 1
    And response message 2 text is "[output of shell::run pruned: was ~2000 tokens; re-call it if still needed]"

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
