@engine @directory @directory_triggers
Feature: directory::engine triggers and registered-triggers (types + instances)
  Trigger TYPES (templates) and registered TRIGGERS (instances) wrap
  `engine::trigger-types::list` and `engine::triggers::list` with the
  same filter / search / worker affordances the function endpoints use.

  Background:
    Given the iii engine is reachable

  # ── triggers::list (trigger types) ─────────────────────────────────

  Scenario: triggers::list includes the directory-published trigger types
    When I call directory::engine::triggers::list with payload:
      """
      {"prefix": "directory::skills::"}
      """
    Then the directory triggers list includes "directory::skills::on-change"

  Scenario: triggers::list worker filter selects matching namespace
    When I call directory::engine::triggers::list with payload:
      """
      {"worker": "directory"}
      """
    Then the directory triggers list includes "directory::skills::on-change"
    And  the directory triggers list includes "directory::prompts::on-change"

  Scenario: search across id and description is case-insensitive
    When I call directory::engine::triggers::list with payload:
      """
      {"search": "ON-CHANGE"}
      """
    Then the directory triggers list includes "directory::skills::on-change"
    And  the directory triggers list includes "directory::prompts::on-change"

  # ── triggers::info ─────────────────────────────────────────────────

  Scenario: triggers::info returns description, worker, and instance count
    When I call directory::engine::triggers::info with payload:
      """
      {"id": "directory::skills::on-change"}
      """
    Then the directory trigger-info response has id "directory::skills::on-change"
    And  the directory trigger-info response has worker_name "directory"
    And  the directory trigger-info response has a non-empty description
    And  the directory trigger-info instance_count is a number

  Scenario: triggers::info errors on an unknown id
    When I call directory::engine::triggers::info with payload:
      """
      {"id": "nope::nope"}
      """
    Then the directory::engine::triggers::info call fails with a message mentioning "not found"

  # ── registered-triggers::list ──────────────────────────────────────

  Scenario: registered-triggers::list responds with an array shape
    When I call directory::engine::registered-triggers::list with payload:
      """
      {}
      """
    Then the directory registered-triggers response has a registered_triggers array

  Scenario: registered-triggers::list respects function_id filter
    When I call directory::engine::registered-triggers::list with payload:
      """
      {"function_id": "no-such::function"}
      """
    Then the directory registered-triggers list is empty

  # ── registered-triggers::info ──────────────────────────────────────

  Scenario: registered-triggers::info errors on an unknown id
    When I call directory::engine::registered-triggers::info with payload:
      """
      {"id": "no-such-id"}
      """
    Then the directory::engine::registered-triggers::info call fails with a message mentioning "not found"
