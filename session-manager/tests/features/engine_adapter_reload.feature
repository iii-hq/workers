@engine
Feature: an adapter hot-reload resyncs the session event observers

  An adapter change (a new data_dir, fs <-> bridge) swaps the store under
  the observers of the session::on-* event functions. The worker replays
  the new store's state through those functions, and calls
  session::on-deleted for every session the previous store held and the
  new one lacks, so an open view converges without polling. The previous
  store's sessions are listed only while session::on-deleted is observed.

  # Prevents: a hot-reload that skips the previous store's sessions while
  # session::on-deleted is observed, which leaves a session the new store
  # lacks in every open view.
  Scenario: an observer of session::on-deleted hears of a session the new store lacks
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And a reloadable session-manager over a directory holding a session "S0" with metadata:
      """
      { "test_run": "${M1}" }
      """
    And an engine observer "R_DELETED" of "session::on-deleted" with filters:
      """
      { "input.session_metadata.test_run": { "$eq": "${M1}" } }
      """
    When the reloadable session-manager's adapter changes to an empty directory
    Then within 10 seconds the engine observer "R_DELETED" receives at least 1 delivery
    And after 1 second the engine observer "R_DELETED" has received exactly 1 delivery
    And every engine delivery to "R_DELETED" has "session_id" = "${S0}"
    And every engine delivery to "R_DELETED" has "session_metadata.test_run" = "${M1}"
