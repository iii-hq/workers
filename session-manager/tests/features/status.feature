@pure
Feature: session::set-status — the coarse lifecycle status

  Contract (session-manager.md § Session status / session::set-status):
  idle -> working -> done/error is driven by the harness; consumers
  render it directly (spinner, done badge, list filter). set_status is
  a NO-OP (no event) when both the status AND the stored reason are
  unchanged; a reason change alone re-emits so UIs can render live
  phase updates within one "working" stretch. reason is stored as
  status_reason — kept on "error" (failure cause) and "working" (phase
  detail), cleared on "idle"/"done" — so a standalone UI can render
  progress and failures without asking the harness.

  Background:
    Given a bare session

  # Prevents: status changes that don't notify (spinners would hang) or
  # that report the wrong previous_status (transition animations break).
  Scenario: a status change fires status_changed with the previous status
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    Then the call succeeds
    And the response field "status" is "working"
    And the response field "previous_status" is "idle"
    And function "ui::status" received 1 "session::status-changed" delivery
    And delivery 0 to "ui::status" has "status" = "working"
    And delivery 0 to "ui::status" has "previous_status" = "idle"
    And delivery 0 to "ui::status" has "session_id" = "s_001"

  # Prevents: duplicate status events on redundant driver calls — the
  # spec promises consumers a no-op fires nothing.
  Scenario: setting the same status again fires nothing
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    Given I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    Then the call succeeds
    And the response field "status" is "working"
    And the response field "previous_status" is "working"
    And function "ui::status" received 1 "session::status-changed" delivery

  # Prevents: invisible in-turn phases — a reason change within one
  # "working" stretch must update the stored reason and re-emit so a
  # UI can walk "preparing context" -> "waiting for <model>" live.
  Scenario: a working-phase reason change updates and re-emits
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    Given I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working", "reason": "preparing context" }
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working", "reason": "waiting for claude" }
      """
    And I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.status_reason" is "waiting for claude"
    And function "ui::status" received 2 "session::status-changed" deliveries
    And delivery 1 to "ui::status" has "status" = "working"
    And delivery 1 to "ui::status" has "previous_status" = "working"
    And delivery 1 to "ui::status" has "status_reason" = "waiting for claude"

  # Prevents: duplicate events on redundant stamps — same status AND
  # same reason together stay a strict no-op.
  Scenario: same status with the same reason is still a no-op
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    Given I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working", "reason": "preparing context" }
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working", "reason": "preparing context" }
      """
    Then the call succeeds
    And function "ui::status" received 1 "session::status-changed" delivery

  # Prevents: failures without a cause — error status must carry the
  # short reason for standalone UIs.
  Scenario: error stores the reason and the event carries it
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "error", "reason": "provider quota exhausted" }
      """
    Then delivery 0 to "ui::status" has "status_reason" = "provider quota exhausted"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.status" is "error"
    And the response field "meta.status_reason" is "provider quota exhausted"

  # Prevents: stale error reasons surviving recovery — a session back at
  # work must not still advertise last week's failure.
  Scenario: leaving error clears the stored reason
    Given I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "error", "reason": "boom" }
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    And I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.status" is "working"
    And the response has no field "meta.status_reason"

  # Prevents: the full driver lifecycle (idle -> working -> done)
  # emitting anything other than exactly one event per real transition.
  Scenario: a full turn lifecycle emits one event per transition
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    And I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "done" }
      """
    Then function "ui::status" received 2 "session::status-changed" deliveries
    And delivery 0 to "ui::status" has "previous_status" = "idle"
    And delivery 1 to "ui::status" has "previous_status" = "working"
    And delivery 1 to "ui::status" has "status" = "done"

  # Prevents: setting status on sessions that don't exist.
  Scenario: set_status on an unknown session is rejected
    When I call "session::set-status" with:
      """
      { "session_id": "s_404", "status": "working" }
      """
    Then the call fails with code "session/not_found"
