@pure
Feature: fs backend persistence — one JSONL file per session, replayed on restart

  The fs backend stores each session as one append-only JSONL file
  under data_dir (encoded session id + ".jsonl"). Every @pure scenario
  in this suite already runs on a real FsStore over a tempdir; this
  feature pins the on-disk contract itself: file-per-session layout,
  hostile-id encoding, deletion, and — the durability story — that a
  brand-new store over the same directory replays to identical state.

  # Prevents: sessions sharing files or files appearing in the wrong
  # place — the file-per-session layout IS the backend's contract.
  Scenario: each session gets its own file
    Given a bare session
    And a bare session
    And a user message "hello" appended to "s_001"
    Then the session store directory contains 2 session files
    And a session file exists for "s_001"
    And a session file exists for "s_002"

  # Prevents: data loss across worker restarts — THE reason this
  # backend exists. A fresh FsStore (empty cache) over the same
  # directory must replay meta, entries, revisions and the active leaf
  # exactly.
  Scenario: a restart replays identical state
    Given a session created with:
      """
      { "title": "durable", "metadata": { "owner": "u_1" } }
      """
    And a user message "one" appended to "s_001"
    And an empty assistant message appended to "s_001"
    And I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "streamed" }] }
      """
    And I call "session::set_status" with:
      """
      { "session_id": "s_001", "status": "done" }
      """
    And I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """

    When the worker restarts with the same data directory

    And I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.title" is "durable"
    And the response field "meta.status" is "done"
    And the response field "meta.message_count" is 2
    And the response field "meta.metadata.owner" is "u_1"
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    Then the response field "entry.revision" is 1
    And the response field "entry.message.content.0.text" is "streamed"
    # The branch switch survived too: appends chain from e_001.
    When I append a user message "after restart" to "s_001"
    Then the response field "parent_id" is "e_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.1.entry_id" is "e_003"

  # Prevents: path traversal / unportable filenames from
  # caller-supplied ensure ids — and proves encoded sessions stay fully
  # usable.
  Scenario: hostile session ids are encoded but work end to end
    When I call "session::ensure" with:
      """
      { "session_id": "../escape/chat 42:?", "title": "hostile" }
      """
    Then the call succeeds
    And the session store directory contains 1 session file
    And a session file exists for "../escape/chat 42:?"
    When I append a user message "still works" to "../escape/chat 42:?"
    And I call "session::messages" with:
      """
      { "session_id": "../escape/chat 42:?" }
      """
    Then the response field "messages" has length 1
    When the worker restarts with the same data directory
    And I call "session::list" with "{}"
    Then the response field "sessions" has length 1
    And the response field "sessions.0.session_id" is "../escape/chat 42:?"

  # Prevents: deleted sessions leaving their file (and history) behind.
  Scenario: deleting a session removes its file
    Given a bare session
    And a user message "doomed" appended to "s_001"
    Then a session file exists for "s_001"
    When I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    Then no session file exists for "s_001"
    And the session store directory contains 0 session files
    # And the deletion survives a restart (no zombie resurrection).
    When the worker restarts with the same data directory
    And I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response is null
