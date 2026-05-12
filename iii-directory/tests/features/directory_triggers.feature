@engine @directory @directory_triggers
Feature: directory::*-trigger functions (types + registered instances)
  Trigger TYPES (templates) and registered TRIGGERS (instances) wrap
  `engine::trigger-types::list` and `engine::triggers::list` with the
  same filter / search / worker affordances the function endpoints use.

  Background:
    Given the iii engine is reachable

  # ── trigger-list (trigger types) ───────────────────────────────────

  Scenario: trigger-list includes the skills-published trigger types
    When I call directory::trigger-list with payload:
      """
      {"prefix": "skills::"}
      """
    Then the directory triggers list includes "skills::on-change"

  Scenario: trigger-list worker filter selects matching namespace
    When I call directory::trigger-list with payload:
      """
      {"worker": "prompts"}
      """
    Then the directory triggers list includes "prompts::on-change"

  Scenario: search across id and description is case-insensitive
    When I call directory::trigger-list with payload:
      """
      {"search": "ON-CHANGE"}
      """
    Then the directory triggers list includes "skills::on-change"
    And  the directory triggers list includes "prompts::on-change"

  # ── trigger-info ───────────────────────────────────────────────────

  Scenario: trigger-info returns description, name, worker, and instance count
    When I call directory::trigger-info with payload:
      """
      {"id": "skills::on-change"}
      """
    Then the directory trigger-info response has id "skills::on-change"
    And  the directory trigger-info response has name "on-change"
    And  the directory trigger-info response has worker_name "skills"
    And  the directory trigger-info response has a non-empty description
    And  the directory trigger-info instance_count is a number

  Scenario: trigger-info errors on an unknown id
    When I call directory::trigger-info with payload:
      """
      {"id": "nope::nope"}
      """
    Then the directory::trigger-info call fails with a message mentioning "not found"

  # ── registered-trigger-list ────────────────────────────────────────

  Scenario: registered-trigger-list responds with an array shape
    When I call directory::registered-trigger-list with payload:
      """
      {}
      """
    Then the directory registered-triggers response has a registered_triggers array

  Scenario: registered-trigger-list respects function_id filter
    When I call directory::registered-trigger-list with payload:
      """
      {"function_id": "no-such::function"}
      """
    Then the directory registered-triggers list is empty

  # ── registered-trigger-info ────────────────────────────────────────

  Scenario: registered-trigger-info errors on an unknown id
    When I call directory::registered-trigger-info with payload:
      """
      {"id": "no-such-id"}
      """
    Then the directory::registered-trigger-info call fails with a message mentioning "not found"
