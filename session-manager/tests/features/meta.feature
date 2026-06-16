@pure
Feature: session::set_meta — update title, description and metadata

  Contract (session-manager.md § session::set_meta): updates
  title/description/metadata (e.g. once a titling worker generates them
  from the first exchange). Does not change status or messages. Fires
  session::meta-updated so consumers render new titles live instead of
  polling. A supplied metadata object REPLACES the stored one.

  Background:
    Given a session created with:
      """
      { "title": "untitled", "description": "", "metadata": { "owner": "u_1", "tag": "keep" } }
      """

  # Prevents: meta updates that don't notify — consumers render titles
  # live off meta_updated, never by polling get.
  Scenario: updating title and description fires meta_updated
    Given a binding "b1" on "session::meta-updated" delivering to "ui::meta" with config:
      """
      {}
      """
    Given the clock advances by 250 ms
    When I call "session::set_meta" with:
      """
      { "session_id": "s_001", "title": "Weather question", "description": "asks about rain" }
      """
    Then the call succeeds
    And the response field "meta.title" is "Weather question"
    And the response field "meta.description" is "asks about rain"
    And the response field "meta.updated_at" is 1000250
    And function "ui::meta" received 1 "session::meta-updated" delivery
    And delivery 0 to "ui::meta" has "session_id" = "s_001"
    And delivery 0 to "ui::meta" has "title" = "Weather question"
    And delivery 0 to "ui::meta" has "description" = "asks about rain"
    And delivery 0 to "ui::meta" has "timestamp" = 1000250

  # Prevents: metadata being merged instead of replaced — stale tenancy
  # keys surviving a replace is a security hazard.
  Scenario: a supplied metadata object replaces the stored one wholesale
    When I call "session::set_meta" with:
      """
      { "session_id": "s_001", "metadata": { "owner": "u_2" } }
      """
    Then the response field "meta.metadata.owner" is "u_2"
    And the response has no field "meta.metadata.tag"

  # Prevents: partial updates clobbering fields that were not supplied.
  Scenario: omitted fields keep their current values
    When I call "session::set_meta" with:
      """
      { "session_id": "s_001", "title": "only title changed" }
      """
    Then the response field "meta.title" is "only title changed"
    And the response field "meta.metadata.owner" is "u_1"
    And the response field "meta.description" is ""

  # Prevents: a no-op set_meta spamming meta_updated events.
  Scenario: a set_meta with no fields is a silent no-op
    Given a binding "b1" on "session::meta-updated" delivering to "ui::meta" with config:
      """
      {}
      """
    When I call "session::set_meta" with:
      """
      { "session_id": "s_001" }
      """
    Then the call succeeds
    And the response field "meta.title" is "untitled"
    And function "ui::meta" received no deliveries

  # Prevents: set_meta bleeding into status or messages.
  Scenario: set_meta never touches status or messages
    Given I call "session::set_status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    And a user message "hi" appended to "s_001"
    When I call "session::set_meta" with:
      """
      { "session_id": "s_001", "title": "T2" }
      """
    Then the response field "meta.status" is "working"
    And the response field "meta.message_count" is 1

  # Prevents: meta_updated filters evaluating against the PRE-update
  # metadata — a consumer subscribed to the new owner must see the
  # event that handed the session over.
  Scenario: metadata filters see the post-update metadata
    Given a binding "b1" on "session::meta-updated" delivering to "obs::u2" with config:
      """
      { "metadata": { "owner": "u_2" } }
      """
    When I call "session::set_meta" with:
      """
      { "session_id": "s_001", "metadata": { "owner": "u_2" } }
      """
    Then function "obs::u2" received 1 delivery
    And delivery 0 to "obs::u2" has "metadata.owner" = "u_2"

  # Prevents: updating sessions that don't exist.
  Scenario: set_meta on an unknown session is rejected
    When I call "session::set_meta" with:
      """
      { "session_id": "s_404", "title": "x" }
      """
    Then the call fails with code "session/not_found"
