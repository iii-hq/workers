@engine
Feature: bridge backend — storage deferral and event propagation

  A bridged instance keeps all domain logic locally but stores through
  the main instance's session::store::* protocol, publishes its events
  to the main (session::store::publish_events), and receives EVERY
  participant's events back through the main's session::store::events
  feed via its relay. The main is the single fan-out point: with
  multiple bridged instances attached, a mutation made anywhere reaches
  every instance's local subscribers exactly once, with each instance
  applying its own binding filters.

  These scenarios build real bridged stacks (BridgeStore +
  RemotePublisher + relay + local Emitter) against the test engine,
  whose in-process fs-mode registration plays the main. Each scenario
  soft-skips when no engine is reachable.

  # Prevents: the bridge writing somewhere other than the main's store —
  # what a bridged instance writes, the main's own session::* surface
  # and JSONL files must see verbatim.
  Scenario: a bridged mutation lands in the main store
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And a bridged instance "B1"
    When via bridged instance "B1" I call "session::create" with:
      """
      { "title": "bridged", "metadata": { "test_run": "${M1}" } }
      """
    Then the call succeeds
    And the response field "meta.status" is "idle"
    And I alias the response field "session_id" as "S1"
    When via bridged instance "B1" I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "b-1",
        "message": { "role": "user", "content": [{ "type": "text", "text": "from the bridge" }], "timestamp": 1 } }
      """
    Then the call succeeds
    And the response field "parent_id" is null
    # Cross-view consistency: the main's own function surface reads the
    # same data the bridge wrote...
    When over the engine I call "session::messages" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response field "messages" has length 1
    And the engine response field "messages.0.message.content.0.text" is "from the bridge"
    # ...and it is durably on the main's disk.
    And the main store file for session "${S1}" exists
    And the latest record for entry "b-1" of session "${S1}" has "message.content.0.text" = "from the bridge"
    And the latest leaf record for session "${S1}" is "b-1"

  # Prevents: bridged domain logic diverging from fs-mode logic — the
  # bridge runs the same service, so idempotent appends and errors must
  # behave identically through the protocol.
  Scenario: idempotency and error codes hold through the bridge
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And a bridged instance "B1"
    Given via bridged instance "B1" I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And I alias the response field "session_id" as "S1"
    When via bridged instance "B1" I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "idem-b",
        "message": { "role": "user", "content": [{ "type": "text", "text": "first" }], "timestamp": 1 } }
      """
    And via bridged instance "B1" I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "idem-b",
        "message": { "role": "user", "content": [{ "type": "text", "text": "REPLAY" }], "timestamp": 2 } }
      """
    Then the call succeeds
    And the response field "entry_id" is "idem-b"
    When via bridged instance "B1" I call "session::get" with:
      """
      { "session_id": "${S1}" }
      """
    Then the response field "meta.message_count" is 1
    When via bridged instance "B1" I call "session::append" with:
      """
      { "session_id": "ghost-session",
        "message": { "role": "user", "content": [{ "type": "text", "text": "into the void" }], "timestamp": 3 } }
      """
    Then the call fails with code "session/not_found"

  # Prevents: THE propagation requirement regressing — with several
  # bridged instances attached, a mutation on one must reach the
  # originator's, every other bridge's, and the main's subscribers,
  # exactly once each, with filters applied at each edge.
  Scenario: a bridged mutation propagates to every instance exactly once
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And a bridged instance "B1"
    And a bridged instance "B2"
    And a local binding "b1" on "session::message-added" in bridged instance "B1" delivering to "ui::b1" with config:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And a local binding "b2" on "session::message-added" in bridged instance "B2" delivering to "ui::b2" with config:
      """
      { "roles": ["user"], "metadata": { "test_run": "${M1}" } }
      """
    And an engine subscriber "R_MAIN" on "session::message-added" with config:
      """
      { "metadata": { "test_run": "${M1}" } }
      """

    When via bridged instance "B1" I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And I alias the response field "session_id" as "S1"
    And via bridged instance "B1" I call "session::append" with:
      """
      { "session_id": "${S1}",
        "message": { "role": "user", "content": [{ "type": "text", "text": "fan out" }], "timestamp": 1 } }
      """
    And via bridged instance "B1" I call "session::append" with:
      """
      { "session_id": "${S1}",
        "message": { "role": "assistant", "content": [], "stop_reason": "end",
                     "model": "m", "provider": "p", "timestamp": 2 } }
      """

    # The originator's local subscriber: both appends, via the round
    # trip through the main (single-path rule, no local short-circuit).
    Then within 10 seconds bridged instance "B1" function "ui::b1" receives at least 2 deliveries
    And every delivery to bridged instance "B1" function "ui::b1" has "session_id" = "${S1}"
    And some delivery to bridged instance "B1" function "ui::b1" has "message.role" = "assistant"

    # The second bridge: roles filter applied at ITS edge — only the
    # user message arrives.
    And within 10 seconds bridged instance "B2" function "ui::b2" receives at least 1 delivery
    And after 1 second bridged instance "B2" function "ui::b2" has received exactly 1 delivery
    And every delivery to bridged instance "B2" function "ui::b2" has "message.role" = "user"

    # The main's own subscriber sees both appends.
    And within 10 seconds the engine subscriber "R_MAIN" receives at least 2 deliveries
    And every engine delivery to "R_MAIN" has "session_id" = "${S1}"

    # No duplicates anywhere: the originator received each event once.
    And after 1 second bridged instance "B1" function "ui::b1" has received exactly 2 deliveries

  # Prevents: main-originated mutations staying invisible to bridged
  # instances — the fan-out must cover events the main produces itself.
  Scenario: a main-side mutation propagates to bridged subscribers
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And a bridged instance "B1"
    Given over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S1"
    And a local binding "b1" on "session::status-changed" in bridged instance "B1" delivering to "ui::b1" with config:
      """
      { "session_id": "${S1}" }
      """
    When over the engine I call "session::set_status" with:
      """
      { "session_id": "${S1}", "status": "working" }
      """
    Then the engine call succeeds
    And within 10 seconds bridged instance "B1" function "ui::b1" receives at least 1 delivery
    And some delivery to bridged instance "B1" function "ui::b1" has "status" = "working"
    And some delivery to bridged instance "B1" function "ui::b1" has "previous_status" = "idle"

  # Prevents: tenancy leaks across the bridge — metadata filters are
  # evaluated at the bridged edge against the envelope's session
  # metadata, so foreign-tenant events must not reach the subscriber.
  Scenario: bridged metadata filters drop foreign-tenant events
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And a fresh unique marker as "M2"
    And a bridged instance "B1"
    And a local binding "b1" on "session::created" in bridged instance "B1" delivering to "ui::tenant1" with config:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    When over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M2}" } }
      """
    And over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    Then within 10 seconds bridged instance "B1" function "ui::tenant1" receives at least 1 delivery
    And after 1 second bridged instance "B1" function "ui::tenant1" has received exactly 1 delivery
