@engine @notifications
Feature: skills::on-change and prompts::on-change fan-out
  Each mutation of the skills / prompts registries fans out through
  the custom trigger types `skills::on-change` and `prompts::on-change`.
  These scenarios register an in-process subscriber, mutate the
  registry, and assert the subscriber observed the expected number of
  events within a bounded window.

  Background:
    Given the iii engine is reachable

  Scenario: skills::register fires skills::on-change
    Given a subscriber to skills::on-change is registered
    When I register a scoped skill with a short body
    Then the subscriber observed at least 1 event within 3000 ms

  Scenario: skills::unregister fires skills::on-change
    Given a subscriber to skills::on-change is registered
    When I register a scoped skill with a short body
    And  I unregister that skill
    Then the subscriber observed at least 2 events within 4000 ms

  Scenario: 3 rapid skill mutations each produce a notification
    Given a subscriber to skills::on-change is registered
    When I register 3 scoped skills in quick succession
    Then the subscriber observed at least 3 events within 5000 ms

  Scenario: prompts::register fires prompts::on-change
    Given a subscriber to prompts::on-change is registered
    When I register a scoped prompt
    Then the subscriber observed at least 1 event within 3000 ms

  Scenario: prompts::unregister fires prompts::on-change
    Given a subscriber to prompts::on-change is registered
    When I register a scoped prompt
    And  I unregister that prompt
    Then the subscriber observed at least 2 events within 4000 ms
