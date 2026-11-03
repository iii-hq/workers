@engine
Feature: the event functions are private and take no args

  session-manager hosts the six session::on-* event functions and the
  internal session::store::on-events feed. Each is private: only the
  connection that hosts it calls it, so no other worker can put a forged
  event in front of its observers, and any connection can observe it. None
  takes args: observers select events with filters on the event's fields,
  and an observer that passes args gets INVALID_ARGS from the engine.

  # Prevents: an event function registered without `.private()` (any
  # worker could call it and reach every observer with a forged event), or
  # with an args_format (observers would pass args the worker never reads).
  Scenario Outline: <function> is private and takes no args
    Given the iii engine is reachable
    Then the engine lists "<function>" as private with no args_format
    And another engine connection can observe "<function>"
    When another engine connection calls "<function>" with:
      """
      { "session_id": "forged", "timestamp": 1 }
      """
    Then the engine call fails with code "FORBIDDEN"
    When an engine observer of "<function>" passes the args:
      """
      { "session_id": "s_1" }
      """
    Then the observer is refused with code "INVALID_ARGS"

    Examples:
      | function                    |
      | session::on-created         |
      | session::on-message-added   |
      | session::on-message-updated |
      | session::on-status-changed  |
      | session::on-meta-updated    |
      | session::on-deleted         |
      | session::store::on-events   |
