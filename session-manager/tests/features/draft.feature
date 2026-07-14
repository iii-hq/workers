@pure
Feature: session::set-draft — park the unsent composer input

  Contract: `session::set-draft` stores the text a user has typed but not
  sent, so a client reload can restore it. It is event-silent and never
  bumps `updated_at` — drafts are written at keystroke cadence, and a save
  must neither re-order `session::list` nor spam `session::meta-updated`
  subscribers. The draft reads back as `meta.draft` on `session::get` /
  `session::list`; `session::set-meta` never touches it. Empty or
  whitespace-only text clears the stored draft.

  Background:
    Given a session created with:
      """
      { "title": "untitled", "metadata": { "owner": "u_1" } }
      """

  # Prevents: losing the user's typed-but-unsent input on reload — THE
  # reason this function exists.
  Scenario: a stored draft reads back on session::get
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "half-typed thought" }
      """
    Then the call succeeds
    And the response field "draft" is "half-typed thought"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.draft" is "half-typed thought"

  # Prevents: keystroke-cadence saves re-ordering session::list (updated_at)
  # or notifying meta-updated subscribers on every keypress.
  Scenario: saving a draft fires no event and keeps updated_at
    Given a binding "b1" on "session::meta-updated" delivering to "ui::meta" with config:
      """
      {}
      """
    Given the clock advances by 250 ms
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "typing…" }
      """
    Then the call succeeds
    And function "ui::meta" received no deliveries
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.updated_at" is 1000000

  # Prevents: an emptied composer leaving a stale draft to reappear on the
  # next reload.
  Scenario: empty or whitespace-only text clears the draft
    Given I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "will be discarded" }
      """
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "   " }
      """
    Then the call succeeds
    And the response field "draft" is null
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response has no field "meta.draft"

  # Prevents: set-meta's wholesale metadata replace bleeding into the draft
  # (they are separate write paths on the same record).
  Scenario: set-meta leaves the stored draft alone
    Given I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "keep me" }
      """
    When I call "session::set-meta" with:
      """
      { "session_id": "s_001", "title": "renamed", "metadata": { "owner": "u_2" } }
      """
    Then the call succeeds
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.draft" is "keep me"
    And the response field "meta.title" is "renamed"

  # Prevents: parking drafts on sessions that don't exist.
  Scenario: set-draft on an unknown session is rejected
    When I call "session::set-draft" with:
      """
      { "session_id": "s_404", "draft": "x" }
      """
    Then the call fails with code "session/not_found"
