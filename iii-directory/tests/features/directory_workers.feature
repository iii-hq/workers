@engine @directory @directory_workers
Feature: directory::worker-list and directory::worker-info
  Wraps `engine::workers::list` with name / runtime / status filters
  and a denormalized `worker-info` view that wraps the worker envelope
  alongside the lists of functions, owned trigger types, and registered
  triggers.

  Background:
    Given the iii engine is reachable

  # ── worker-list ────────────────────────────────────────────────────

  Scenario: worker-list returns at least one worker (the test client)
    When I call directory::worker-list with payload:
      """
      {}
      """
    Then the directory workers response has a workers array
    And  the directory workers list is non-empty

  Scenario: worker-list status filter rejects mismatched values
    When I call directory::worker-list with payload:
      """
      {"status": "definitely-not-a-real-status"}
      """
    Then the directory workers list is empty

  Scenario: every worker entry exposes id and status
    When I call directory::worker-list with payload:
      """
      {}
      """
    Then every directory workers list entry has a non-empty id
    And  every directory workers list entry has a non-empty status

  Scenario: every worker entry has the shared description field (always null for directory)
    When I call directory::worker-list with payload:
      """
      {}
      """
    Then every directory workers list entry has a null description

  # ── worker-info ────────────────────────────────────────────────────

  Scenario: worker-info errors on an unknown name
    When I call directory::worker-info with payload:
      """
      {"name": "no-such-worker-12345"}
      """
    Then the directory::worker-info call fails with a message mentioning "not found"

  Scenario: worker-info rejects an empty name
    When I call directory::worker-info with payload:
      """
      {"name": "  "}
      """
    Then the directory::worker-info call fails with a message mentioning "non-empty"
