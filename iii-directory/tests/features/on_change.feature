@engine @on_change
Feature: change functions (directory::skills::on-change, directory::system-prompts::on-change)
  Each write through this worker calls the matching change function once it
  lands. The functions are private, take no args, and return the event, so
  an observer with `deliver: output` receives the event itself; filters on
  the event's fields select which writes reach it.

  Background:
    Given the iii engine is reachable

  Scenario: a skill create, update and delete each call directory::skills::on-change
    Given an observer of "directory::skills::on-change" delivering to "bdd::skills-changed"
    When I create the skill "team-c/notes" with body:
      """
      # Notes

      First version.
      """
    And I update the skill "team-c/notes" with body:
      """
      # Notes

      Second version.
      """
    And I delete the skill "team-c/notes"
    Then "bdd::skills-changed" receives 3 events
    And "bdd::skills-changed" received an event with op "create", namespace "team-c", name "team-c/notes" and id "team-c/notes"
    And "bdd::skills-changed" received an event with op "update", namespace "team-c", name "team-c/notes" and id "team-c/notes"
    And "bdd::skills-changed" received an event with op "delete", namespace "team-c", name "team-c/notes" and id "team-c/notes"

  Scenario: a filter on input.name selects one skill's changes
    Given an observer of "directory::skills::on-change" filtered on "input.name" equal to "team-c/kept" delivering to "bdd::skills-kept"
    And an observer of "directory::skills::on-change" filtered on "input.op" equal to "delete" delivering to "bdd::skills-deleted"
    When I create the skill "team-c/kept" with body:
      """
      # Kept

      Watched.
      """
    And I create the skill "team-c/other" with body:
      """
      # Other

      Not watched.
      """
    Then "bdd::skills-kept" receives 1 event
    And "bdd::skills-kept" received an event with op "create", namespace "team-c", name "team-c/kept" and id "team-c/kept"
    And "bdd::skills-deleted" receives nothing

  Scenario: a system prompt create calls directory::system-prompts::on-change with its name
    Given an observer of "directory::system-prompts::on-change" delivering to "bdd::prompts-changed"
    And an observer of "directory::skills::on-change" delivering to "bdd::skills-quiet"
    When I create the system prompt "bdd-change-reviewer" with body:
      """
      ---
      description: Reviews changes.
      ---
      You review changes.
      """
    Then "bdd::prompts-changed" receives 1 event
    And "bdd::prompts-changed" received an event with op "create" and name "bdd-change-reviewer"
    And "bdd::skills-quiet" receives nothing
