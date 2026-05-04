@engine @skills_register
Feature: skills::register / unregister / list
  The state-backed skills registry validates ids + bodies up front,
  writes to the `skills` scope, fans out through the
  `skills::on-change` trigger type, and supports idempotent removal.

  Background:
    Given the iii engine is reachable

  # ── id validation ─────────────────────────────────────────────────────

  Scenario: register rejects an empty id
    When I register a skill with id "" and body "# hi"
    Then the skills::register call fails
    And  the skills::register error mentions "id must be non-empty"

  Scenario: register rejects an uppercase id
    When I register a skill with id "Bad-Id" and body "# hi"
    Then the skills::register call fails
    And  the skills::register error mentions "lowercase"

  Scenario: register rejects an id with a space
    When I register a skill with id "with space" and body "# hi"
    Then the skills::register call fails

  Scenario: register rejects an id with a slash
    When I register a skill with id "with/slash" and body "# hi"
    Then the skills::register call fails

  Scenario: register rejects an id with a colon
    When I register a skill with id "with::colon" and body "# hi"
    Then the skills::register call fails

  Scenario: register rejects a 65-char id
    When I register a skill with id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" and body "# ok"
    Then the skills::register call fails
    And  the skills::register error mentions "too long"

  # ── body validation ───────────────────────────────────────────────────

  Scenario: register rejects an empty body
    When I register a skill with id "emptyid" and an empty body
    Then the skills::register call fails
    And  the skills::register error mentions "non-empty"

  Scenario: register rejects a body bigger than 256 KiB
    When I register a skill with id "bigid" and a body of 262145 bytes
    Then the skills::register call fails
    And  the skills::register error mentions "too large"

  Scenario: register accepts a body right at 256 KiB
    When I register a skill with id "maxid" and a body of 262144 bytes
    Then the skills::register call succeeds

  # ── round-trip ────────────────────────────────────────────────────────

  Scenario: register then list shows the scoped skill
    When I register a scoped skill "smoke" with body "# smoke\n\nsmoke body"
    Then the skills::register call succeeds
    When I list skills
    Then the scoped skill appears in the listing
    And  the listing entries carry bytes and registered_at but no skill body
    And  the listing is sorted by id

  Scenario: re-register overwrites the body and refreshes the timestamp
    When I register a scoped skill "twice" with body "# one\nfirst"
    And  I record the registered_at timestamp
    And  I re-register the scoped skill with body "# one\nsecond"
    Then the skills::register call succeeds
    And  the re-registered timestamp is different from the first

  Scenario: unregister is idempotent
    When I register a scoped skill "gone" with body "# gone\nbody"
    And  I unregister the scoped skill
    Then the last unregister returned removed=true
    When I unregister the scoped skill again
    Then the last unregister returned removed=false
    When I list skills
    Then the scoped skill does not appear in the listing
