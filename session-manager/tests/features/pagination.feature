@pure
Feature: lazy transcript readers — open a long session a page at a time

  Contract: `session::messages-tail` returns the NEWEST page of the active
  path and walks backwards from there (`before_entry_id`), counting pages in
  blocks: a plain entry, or one whole *activity run* — assistant messages
  that call functions, the results answering them, and the trigger wakes
  that led to them. A page never cuts inside a run. Inside a run only the
  last call, its results and the wakes come back whole; the rest is
  `elided: true` placeholders that keep identity (call ids, function ids,
  text) but drop arguments, result bodies, details and thinking.
  `session::messages-range` brings any of those back in full, by span or by
  id list. Nothing is ever removed from storage: `session::messages` keeps
  returning every entry in full, and both readers are pure read-time views.

  Background:
    Given a bare session

  # Prevents: a chat UI downloading a 2,000-entry transcript to show the
  # last three exchanges — THE reason this reader exists.
  Scenario: the newest page comes first and pages walk backwards by block
    Given a user message "one" appended to "s_001"
    And an assistant call "c1" to "shell::run" appended to "s_001"
    And a function result for "c1" appended to "s_001"
    And a final assistant message "done" appended to "s_001"
    And a user message "two" appended to "s_001"
    And a final assistant message "done again" appended to "s_001"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 2 }
      """
    Then the call succeeds
    And the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_005"
    And the response field "messages.1.entry_id" is "e_006"
    And the response field "has_more" is true
    And the response field "oldest_entry_id" is "e_005"
    # Two blocks before e_005: the run (e_002+e_003) and the prose e_004.
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 2, "before_entry_id": "e_005" }
      """
    Then the response field "messages" has length 3
    And the response field "messages.0.entry_id" is "e_002"
    And the response field "messages.1.entry_id" is "e_003"
    And the response field "messages.2.entry_id" is "e_004"
    And the response field "has_more" is true
    And the response field "oldest_entry_id" is "e_002"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 2, "before_entry_id": "e_002" }
      """
    Then the response field "messages" has length 1
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "has_more" is false

  # Prevents: a page boundary landing between a call and its result, or in
  # the middle of the run a reader collapses as one group.
  Scenario: a tool run is never split across pages, whatever the limit
    Given a user message "go" appended to "s_001"
    And a tool run of 3 calls named "c" appended to "s_001"
    And a final assistant message "done" appended to "s_001"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 1 }
      """
    Then the response field "messages" has length 1
    And the response field "messages.0.entry_id" is "e_008"
    And the response field "has_more" is true
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 1, "before_entry_id": "e_008" }
      """
    Then the response field "messages" has length 6
    And the response field "messages.0.entry_id" is "e_002"
    And the response field "messages.5.entry_id" is "e_007"
    And the response field "has_more" is true
    And the response field "oldest_entry_id" is "e_002"

  # Prevents: the walk back drifting from the truth — a gap, a duplicate or
  # a reordering between what the pages add up to and the full read.
  Scenario: paging back to the root reproduces session::messages exactly
    Given a user message "one" appended to "s_001"
    And a tool run of 4 calls named "a" appended to "s_001"
    And an assistant call "a_5" to "shell::run" saying "almost" appended to "s_001"
    And a function result for "a_5" appended to "s_001"
    And a final assistant message "done" appended to "s_001"
    And a custom entry of type "compaction" appended to "s_001"
    And a user message "two" appended to "s_001"
    And a tool run of 2 calls named "b" appended to "s_001"
    And a final assistant message "done again" appended to "s_001"
    Then walking "s_001" back through "session::messages-tail" with limit 1 matches "session::messages"
    And walking "s_001" back through "session::messages-tail" with limit 2 matches "session::messages"
    And walking "s_001" back through "session::messages-tail" with limit 50 matches "session::messages"

  # Prevents: a collapsed run arriving with every argument and result body
  # anyway — and, the other way round, a placeholder that lost what the
  # reader needs to draw it and to swap the full entry in later.
  Scenario: inside a run only the last call and its result come back whole
    Given a user message "q" appended to "s_001"
    And an assistant call "c1" to "shell::run" saying "step one" appended to "s_001"
    And a function result for "c1" appended to "s_001"
    And an assistant call "c2" to "shell::run" appended to "s_001"
    And a function result for "c2" appended to "s_001"
    And a final assistant message "done" appended to "s_001"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 6
    And the response has no field "messages.0.elided"
    # First call: a placeholder. Thinking gone, text kept, call keeps its
    # identity, arguments emptied.
    And the response field "messages.1.elided" is true
    And the response field "messages.1.message.content" has length 2
    And the response field "messages.1.message.content.0.text" is "step one"
    And the response field "messages.1.message.content.1.id" is "c1"
    And the response field "messages.1.message.content.1.function_id" is "shell::run"
    And the response field "messages.1.message.content.1.arguments" is {}
    And the response field "messages.1.message.stop_reason" is "function_call"
    # Its result: pairing and status kept, body and details gone.
    And the response field "messages.2.elided" is true
    And the response field "messages.2.message.function_call_id" is "c1"
    And the response field "messages.2.message.is_error" is false
    And the response field "messages.2.message.content" has length 0
    And the response field "messages.2.message.details" is null
    # Last call and its result: whole, thinking included.
    And the response has no field "messages.3.elided"
    And the response field "messages.3.message.content.0.type" is "thinking"
    And the response field "messages.3.message.content.1.arguments.cmd" is "ls"
    And the response has no field "messages.4.elided"
    And the response field "messages.4.message.content.0.text" is "README.md"
    And the response field "messages.4.message.details.code" is 0
    And the response has no field "messages.5.elided"
    # Storage is untouched: the classic reader still has everything.
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages.1.message.content" has length 3
    And the response field "messages.1.message.content.2.arguments.cmd" is "ls"
    And the response field "messages.2.message.content.0.text" is "README.md"

  # Prevents: "show all" having no way to get the placeholders' content —
  # and a partial answer for a long run with no way to continue.
  Scenario: messages-range returns a span in full, in pages
    Given a user message "q" appended to "s_001"
    And a tool run of 3 calls named "c" appended to "s_001"
    And a final assistant message "done" appended to "s_001"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "from_entry_id": "e_002", "to_entry_id": "e_007", "limit": 4 }
      """
    Then the call succeeds
    And the response field "messages" has length 4
    And the response field "messages.0.entry_id" is "e_002"
    And the response has no field "messages.0.elided"
    And the response field "messages.0.message.content.1.arguments.cmd" is "ls"
    And the response field "messages.1.message.content.0.text" is "README.md"
    And the response field "messages.3.entry_id" is "e_005"
    And I alias the response field "next_cursor" as "C1"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "from_entry_id": "e_002", "to_entry_id": "e_007", "limit": 4, "cursor": "${C1}" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_006"
    And the response field "messages.1.entry_id" is "e_007"
    And the response has no field "next_cursor"

  # Prevents: a renderer that must draw a call even while its group is
  # collapsed (a registered trigger, a spawned agent) having to pull the
  # whole run to get it.
  Scenario: messages-range returns specific entries in path order
    Given a user message "q" appended to "s_001"
    And a tool run of 3 calls named "c" appended to "s_001"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "entry_ids": ["e_006", "e_002", "e_006"] }
      """
    Then the call succeeds
    And the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_002"
    And the response field "messages.1.entry_id" is "e_006"
    And the response has no field "next_cursor"

  # Prevents: a malformed selection quietly answering with the wrong slice.
  Scenario: messages-range rejects bad selections
    Given a user message "q" appended to "s_001"
    And a user message "r" appended to "s_001"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "from_entry_id": "e_002", "to_entry_id": "e_001" }
      """
    Then the call fails with code "session/invalid_request"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "from_entry_id": "e_001" }
      """
    Then the call fails with code "session/invalid_request"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "entry_ids": ["e_404"] }
      """
    Then the call fails with code "session/entry_not_found"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_404", "entry_ids": ["e_001"] }
      """
    Then the call fails with code "session/not_found"

  # Prevents: a "go to message" link into deep history needing N round
  # trips — one call widens the page until the target is in it.
  Scenario: until_entry_id widens the page back to the target's block
    Given a user message "one" appended to "s_001"
    And a tool run of 2 calls named "a" appended to "s_001"
    And a final assistant message "done" appended to "s_001"
    And a user message "two" appended to "s_001"
    And a final assistant message "done again" appended to "s_001"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 1, "until_entry_id": "e_003" }
      """
    Then the response field "messages" has length 7
    And the response field "messages.0.entry_id" is "e_002"
    And the response field "has_more" is true
    # A target already inside the page changes nothing.
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "limit": 2, "until_entry_id": "e_008" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_007"

  # Prevents: a stale anchor (the leaf moved under the reader) being paged
  # from silently, instead of the reload signal `session::messages` gives.
  Scenario: anchors off the active path are rejected as stale cursors
    Given a user message "one" appended to "s_001"
    And a user message "two" appended to "s_001"
    When I call "session::set-active-leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    And I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "before_entry_id": "e_002" }
      """
    Then the call fails with code "session/invalid_cursor"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "until_entry_id": "e_002" }
      """
    Then the call fails with code "session/invalid_cursor"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "entry_ids": ["e_002"] }
      """
    Then the call fails with code "session/invalid_cursor"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_404" }
      """
    Then the call fails with code "session/not_found"

  # Prevents: bookkeeping entries vanishing from the reader that draws them
  # in place (compaction markers), while still allowing a messages-only read.
  Scenario: custom entries are included by default and can be dropped
    Given a user message "one" appended to "s_001"
    And a custom entry of type "compaction" appended to "s_001"
    And a user message "two" appended to "s_001"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 3
    And the response field "messages.1.custom.custom_type" is "compaction"
    And the response has no field "messages.1.elided"
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "include_custom": false }
      """
    Then the response field "messages" has length 2
    And the response field "messages.1.entry_id" is "e_003"

  # Prevents: the lazy reader re-inflating the images the eager one learned
  # to leave out.
  Scenario: both readers honour include_image_data
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "user", "timestamp": 1, "content": [
        { "type": "image", "mime": "image/png", "data": "aGVsbG8=", "attachment_id": "a_001" }
      ] } }
      """
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001", "include_image_data": false }
      """
    Then the response field "messages.0.message.content.0.data" is ""
    And the response field "messages.0.message.content.0.attachment_id" is "a_001"
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "entry_ids": ["e_001"], "include_image_data": false }
      """
    Then the response field "messages.0.message.content.0.data" is ""
    When I call "session::messages-range" with:
      """
      { "session_id": "s_001", "entry_ids": ["e_001"] }
      """
    Then the response field "messages.0.message.content.0.data" is "aGVsbG8="

  # Prevents: a brand-new session erroring instead of answering "nothing
  # yet".
  Scenario: an empty session has an empty newest page
    When I call "session::messages-tail" with:
      """
      { "session_id": "s_001" }
      """
    Then the call succeeds
    And the response field "messages" has length 0
    And the response field "has_more" is false
    And the response has no field "oldest_entry_id"
