@pure
Feature: compaction leases — one summariser per logical session

  Contract (context-manager.md § State): scope `context_lease`, key =
  options.lease_key (e.g. a session id) or a hash of the message set;
  value {nonce, ts}; TTL ~300s. Two callers must not summarise the same
  logical session concurrently; crashed holders expire via TTL; a
  holder must never clear someone else's claim.

  Background:
    Given the router knows model "big" with context window 200000 and max output 8000
    And a user message of ~9000 tokens
    And a user message "q2"
    And a user message "q3"

  # Prevents: two harness instances double-summarising one session —
  # the second caller must see busy and must NOT consume an LLM call.
  Scenario: a held lease turns the second caller away as busy
    Given a foreign lease on the history's default key taken 0 ms ago
    When I compact the history with model "big"
    Then the call succeeds
    And the response field "status" is "busy"
    And the summariser was never invoked
    And the lease on the history's default key still belongs to the foreign holder

  # Prevents: a crashed holder locking a session forever — claims older
  # than the TTL must read as free.
  Scenario: an expired lease is taken over
    Given a foreign lease on the history's default key taken 0 ms ago
    And the clock advances by 300001 ms
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And no lease claim remains

  # Prevents: a lease one millisecond short of the TTL being treated as
  # expired — early takeover is the double-summarisation bug again.
  Scenario: a lease just inside the TTL still wins
    Given a foreign lease on the history's default key taken 0 ms ago
    And the clock advances by 299999 ms
    When I compact the history with model "big"
    Then the response field "status" is "busy"

  # Prevents: callers with an explicit session key colliding with (or
  # missing) each other — the same lease_key must contend.
  Scenario: an explicit lease_key contends with its holder
    Given a foreign lease on key "session-42" taken 0 ms ago
    When I compact the history with model "big" and options:
      """
      { "lease_key": "session-42" }
      """
    Then the response field "status" is "busy"
    And the lease on key "session-42" still belongs to "foreign-holder"

  # Prevents: unrelated histories blocking each other — the default
  # key is a hash of THIS message set, so a foreign claim on some other
  # session must not contend.
  Scenario: different histories never contend by default
    Given a foreign lease on key "some-other-session" taken 0 ms ago
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And the lease on key "some-other-session" still belongs to "foreign-holder"

  # Prevents: a successful compaction leaving its claim behind and
  # blocking every later compaction until TTL.
  Scenario: success releases the lease
    When I compact the history with model "big"
    Then the response field "status" is "ok"
    And no lease claim remains

  # Prevents: a state outage letting every contender through at once —
  # an unverifiable claim must read as busy, never as a win.
  Scenario: a state outage reads as busy, never as a win
    Given the lease store is unavailable
    When I compact the history with model "big"
    Then the call succeeds
    And the response field "status" is "busy"
    And the summariser was never invoked
