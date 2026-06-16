@engine
Feature: engine roundtrip — every function over the bus, persisted as JSONL

  These scenarios drive the registered production surface through a
  LIVE iii engine: bus call -> typed handler -> service -> FsStore ->
  response. Persistence is verified by reading the per-session JSONL
  files back from the main instance's data_dir, pinning the fs
  backend's record contract (typed meta / entry / leaf lines,
  last-wins replay).

  Every scenario soft-skips when no engine is reachable (start iii to
  run them; ids are real UUIDs so aliases carry them between steps).
  Caution: a gherkin description line must never START with the word
  "Scenario" — the parser reads it as a keyword and rejects the file.

  # Prevents: the worker working in-memory but never actually landing
  # records in its JSONL files (silent durability loss).
  Scenario: lifecycle calls persist into the session's file
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    When over the engine I call "session::create" with:
      """
      { "title": "engine roundtrip", "metadata": { "test_run": "${M1}" } }
      """
    Then the engine call succeeds
    And the engine response field "meta.status" is "idle"
    And over the engine I alias the response field "session_id" as "S1"
    And the main store file for session "${S1}" exists
    And the latest meta record for session "${S1}" has "title" = "engine roundtrip"

    When over the engine I call "session::set-meta" with:
      """
      { "session_id": "${S1}", "title": "renamed live" }
      """
    Then the engine call succeeds
    And the latest meta record for session "${S1}" has "title" = "renamed live"

    When over the engine I call "session::set-status" with:
      """
      { "session_id": "${S1}", "status": "working" }
      """
    Then the engine response field "previous_status" is "idle"
    And the latest meta record for session "${S1}" has "status" = "working"

    When over the engine I call "session::list" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    Then the engine response field "sessions" has length 1
    And the engine response field "sessions.0.session_id" is "${S1}"

    When over the engine I call "session::get" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response field "meta.title" is "renamed live"

  # Prevents: entries or the active-leaf pointer landing as malformed
  # records, and bus-level idempotent appends regressing (the replay
  # must not append a second record for the same entry id).
  Scenario: appends and updates persist entries and move the active leaf
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    Given over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S1"

    When over the engine I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "idem-1",
        "message": { "role": "user", "content": [{ "type": "text", "text": "hello engine" }], "timestamp": 1 } }
      """
    Then the engine call succeeds
    And the engine response field "entry_id" is "idem-1"
    And the engine response field "parent_id" is null
    And the latest record for entry "idem-1" of session "${S1}" has "message.content.0.text" = "hello engine"
    And the latest leaf record for session "${S1}" is "idem-1"

    # idempotent replay over the bus: same entry_id, no duplicate
    When over the engine I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "idem-1",
        "message": { "role": "user", "content": [{ "type": "text", "text": "REPLAY" }], "timestamp": 2 } }
      """
    Then the engine response field "entry_id" is "idem-1"
    And the latest record for entry "idem-1" of session "${S1}" has "message.content.0.text" = "hello engine"
    And the latest meta record for session "${S1}" has "message_count" = 1

    When over the engine I call "session::update-message" with:
      """
      { "session_id": "${S1}", "entry_id": "idem-1",
        "content": [{ "type": "text", "text": "edited" }], "expected_revision": 0 }
      """
    Then the engine response field "updated" is true
    And the engine response field "revision" is 1
    And the latest record for entry "idem-1" of session "${S1}" has "revision" = 1
    And the latest record for entry "idem-1" of session "${S1}" has "message.content.0.text" = "edited"

    When over the engine I call "session::messages" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response field "messages" has length 1
    And the engine response field "messages.0.message.content.0.text" is "edited"

    When over the engine I call "session::get-message" with:
      """
      { "session_id": "${S1}", "entry_id": "idem-1" }
      """
    Then the engine response field "entry.kind" is "message"

  # Prevents: fork sharing a file with the source, and delete leaving
  # the session file (or any orphaned records) behind.
  Scenario: fork copies state and delete removes the file
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    Given over the engine I call "session::create" with:
      """
      { "title": "source", "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S1"
    And over the engine I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "root-1",
        "message": { "role": "user", "content": [{ "type": "text", "text": "shared past" }], "timestamp": 1 } }
      """

    When over the engine I call "session::fork" with:
      """
      { "session_id": "${S1}", "entry_id": "root-1", "title": "the fork" }
      """
    Then the engine call succeeds
    And the engine response field "meta.forked_from" is "${S1}"
    And the engine response field "meta.message_count" is 1
    And over the engine I alias the response field "session_id" as "S2"
    And the main store file for session "${S2}" exists

    When over the engine I call "session::ensure" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response field "created" is false

    When over the engine I call "session::delete" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response field "deleted" is true
    And the main store file for session "${S1}" does not exist

    When over the engine I call "session::get" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response is null

    When over the engine I call "session::messages" with:
      """
      { "session_id": "${S2}" }
      """
    Then the engine response field "messages" has length 1
    And the engine response field "messages.0.message.content.0.text" is "shared past"

  # Prevents: error codes losing their stable shape across the bus
  # (callers branch on the session/* prefix programmatically).
  Scenario: bus errors carry the stable session/* codes
    Given the iii engine is reachable
    When over the engine I call "session::set-status" with:
      """
      { "session_id": "definitely-not-a-session", "status": "working" }
      """
    Then the engine call fails with code "session/not_found"
    When over the engine I call "session::append" with:
      """
      { "session_id": "definitely-not-a-session" }
      """
    Then the engine call fails with code "session/not_found"
