@pure
Feature: session::set-status — the coarse lifecycle status

  Contract (session-manager.md § Session status / session::set-status):
  idle -> working -> done/error is driven by the harness; consumers
  render it directly (spinner, done badge, list filter). set_status is
  a NO-OP (no event) when the status is unchanged. reason is stored as
  status_reason — kept on "error", cleared on any other status — so a
  standalone UI can render failures without asking the harness.

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

  # Prevents: spec-strict regression — same status with a DIFFERENT
  # reason is still a no-op; the stored reason must not silently change
  # and the second call must not emit a duplicate status_changed.
  Scenario: same status with a different reason is still a no-op
    Given a binding "b1" on "session::status-changed" delivering to "ui::status" with config:
      """
      {}
      """
    Given I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "error", "reason": "rate limited" }
      """
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "error", "reason": "DIFFERENT" }
      """
    And I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.status_reason" is "rate limited"
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

  # Prevents: the "waiting" lifecycle status (a turn parked awaiting a user
  # decision, e.g. the harness reached max_turns) being rejected by the enum.
  Scenario: a turn can be parked in the waiting status
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
      { "session_id": "s_001", "status": "waiting", "reason": "max_turns" }
      """
    And I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.status" is "waiting"
    And delivery 1 to "ui::status" has "status" = "waiting"
    And delivery 1 to "ui::status" has "previous_status" = "working"

  # Prevents: setting status on sessions that don't exist.
  Scenario: set_status on an unknown session is rejected
    When I call "session::set-status" with:
      """
      { "session_id": "s_404", "status": "working" }
      """
    Then the call fails with code "session/not_found"
