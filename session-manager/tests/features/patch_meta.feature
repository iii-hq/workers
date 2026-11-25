@pure
Feature: session::patch-meta — merge metadata keys on the server

  Contract (chat-projects spec §1.2.1): `set` replaces each named top-level
  metadata key whole, `unset` removes keys, and every key the patch does not
  name stays as stored. The merge runs under the session's write lock and
  calls session::on-meta-updated once with the merged map. A key in both
  `set` and `unset` is refused. session::set-meta keeps its replace-all
  behaviour.

  Background:
    Given a session created with:
      """
      { "title": "untitled", "description": "", "metadata": { "owner": "u_1", "tag": "keep", "fs_scope": { "root": "/w" } } }
      """

  # Prevents: a writer that changes one key erasing the keys it never read.
  Scenario: set and unset touch only the keys they name
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "project_id": "prj_1", "owner": "u_2" }, "unset": ["tag"] }
      """
    Then the call succeeds
    And the response field "meta.metadata.project_id" is "prj_1"
    And the response field "meta.metadata.owner" is "u_2"
    And the response field "meta.metadata.fs_scope.root" is "/w"
    And the response has no field "meta.metadata.tag"
    And the response field "meta.title" is "untitled"

  # Prevents: a set value merged into the stored object instead of replacing it.
  Scenario: a set key is replaced whole
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "fs_scope": { "grants": [] } } }
      """
    Then the response has no field "meta.metadata.fs_scope.root"
    And the response field "meta.metadata.fs_scope.grants" has length 0

  # Prevents: consumers rendering from a partial map (the event must carry the merged one).
  Scenario: one session::on-meta-updated carries the merged metadata
    Given an observer "b1" of "session::on-meta-updated" delivering to "ui::meta"
    Given the clock advances by 250 ms
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "description": "Fixing the login form", "set": { "description_by": "agent" } }
      """
    Then the response field "meta.description" is "Fixing the login form"
    And the response field "meta.updated_at" is 1000250
    And function "ui::meta" received 1 "session::on-meta-updated" delivery
    And delivery 0 to "ui::meta" has "description" = "Fixing the login form"
    And delivery 0 to "ui::meta" has "metadata.description_by" = "agent"
    And delivery 0 to "ui::meta" has "metadata.owner" = "u_1"
    And delivery 0 to "ui::meta" has "metadata.tag" = "keep"

  # Prevents: membership writes (a conversation moved into a project)
  # re-ordering lists sorted by updated_at (chat-projects D50).
  Scenario: touch false keeps updated_at and still emits once
    Given an observer "b1" of "session::on-meta-updated" delivering to "ui::meta"
    Given the clock advances by 250 ms
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "project_id": "prj_1" }, "touch": false }
      """
    Then the call succeeds
    And the response field "meta.metadata.project_id" is "prj_1"
    And the response field "meta.updated_at" is 1000000
    And function "ui::meta" received 1 "session::on-meta-updated" delivery
    And delivery 0 to "ui::meta" has "metadata.project_id" = "prj_1"
    And delivery 0 to "ui::meta" has "metadata.owner" = "u_1"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.updated_at" is 1000000
    And the response field "meta.metadata.project_id" is "prj_1"

  # Prevents: the default changing set-meta's "a write is activity" rule.
  Scenario: the default touch bumps updated_at
    Given the clock advances by 250 ms
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "project_id": "prj_1" }, "touch": true }
      """
    Then the response field "meta.updated_at" is 1000250

  # Prevents: a quiet no-op still announcing itself.
  Scenario: a touch false patch that changes nothing is silent
    Given an observer "b1" of "session::on-meta-updated" delivering to "ui::meta"
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "owner": "u_1" }, "touch": false }
      """
    Then the call succeeds
    And function "ui::meta" received no deliveries

  # Prevents: an ambiguous patch silently picking a winner.
  Scenario: a key in both set and unset is refused
    Given an observer "b1" of "session::on-meta-updated" delivering to "ui::meta"
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "tag": "x" }, "unset": ["tag"] }
      """
    Then the call fails with code "session/invalid_request"
    And function "ui::meta" received no deliveries

  # Prevents: event spam from a patch that writes what is already stored.
  Scenario: a patch that changes nothing is a silent no-op
    Given an observer "b1" of "session::on-meta-updated" delivering to "ui::meta"
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "tag": "keep" }, "unset": ["missing"] }
      """
    Then the call succeeds
    And the response field "meta.metadata.tag" is "keep"
    And function "ui::meta" received no deliveries

  # Prevents: patching sessions that don't exist.
  Scenario: patch-meta on an unknown session is rejected
    When I call "session::patch-meta" with:
      """
      { "session_id": "s_404", "set": { "tag": "x" } }
      """
    Then the call fails with code "session/not_found"

  # Prevents: patch-meta changing set-meta's contract.
  Scenario: set-meta still replaces the whole map after a patch
    Given I call "session::patch-meta" with:
      """
      { "session_id": "s_001", "set": { "project_id": "prj_1" } }
      """
    When I call "session::set-meta" with:
      """
      { "session_id": "s_001", "metadata": { "owner": "u_3" } }
      """
    Then the response field "meta.metadata.owner" is "u_3"
    And the response has no field "meta.metadata.project_id"
    And the response has no field "meta.metadata.tag"
