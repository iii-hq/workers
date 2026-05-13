@engine @read
Feature: filesystem-backed reads (directory::skills::list / directory::skills::get)
  Both read paths source from `skills_folder` on disk. Files arrive
  there via `directory::skills::download` (or by direct editing in
  tests). Scans derive ids from the path relative to `skills_folder`
  with `.md` stripped and `prompts/` segments excluded. The previous
  `iii://` URI scheme (rendered tree, function-backed sections, and
  batched fetch) is gone — bodies are read solely by id via
  `directory::skills::get` and enumerated via
  `directory::skills::list` (each row already carries title +
  description so no follow-up `get` is needed for a picker).

  Background:
    Given the iii engine is reachable

  # ── flat folder of skills ────────────────────────────────────────────

  Scenario: a flat folder of skills appears in skills::list
    Given a skill file at "ns/alpha.md" with body:
      """
      # Alpha

      The alpha skill body.
      """
    And   a skill file at "ns/beta.md" with body:
      """
      # Beta

      The beta skill body.
      """
    When I list skills
    Then the listing has an entry with id "ns/alpha"
    And  the listing has an entry with id "ns/beta"

  Scenario: list rows carry title and description from the body
    Given a skill file at "ns/labelled.md" with body:
      """
      # Labelled skill

      First paragraph summary.

      Second paragraph ignored.
      """
    When I list skills
    Then the listing entry "ns/labelled" has title "Labelled skill"
    And  the listing entry "ns/labelled" has description "First paragraph summary."

  # ── nested directory hierarchy ───────────────────────────────────────

  Scenario: nested folders derive slashed ids
    Given a skill file at "team-a/playbook.md" with body:
      """
      # Team A playbook

      Top-level body.
      """
    And   a skill file at "team-a/meetings/standup.md" with body:
      """
      # Standup

      Nested body.
      """
    When I list skills
    Then the listing has an entry with id "team-a/playbook"
    And  the listing has an entry with id "team-a/meetings/standup"

  # ── directory::skills::get ───────────────────────────────────────────

  Scenario: directory::skills::get returns the body, id, title, description, and modified_at
    Given a skill file at "ns/lookup.md" with body:
      """
      # Lookup

      Body content here.
      """
    When I get skill "ns/lookup"
    Then the get response has id "ns/lookup"
    And  the get response has title "Lookup"
    And  the get response has description "Body content here."
    And  the get response body contains "Body content here."
    And  the get response has a non-empty modified_at

  Scenario: directory::skills::get accepts the legacy iii:// prefix
    Given a skill file at "ns/prefixed.md" with body:
      """
      # Prefixed

      Body for prefixed lookup.
      """
    When I get skill "iii://ns/prefixed"
    Then the get response has id "ns/prefixed"
    And  the get response body contains "Body for prefixed lookup."

  Scenario: file changes between reads are reflected immediately
    Given a skill file at "ns/live.md" with body:
      """
      # Live

      First version.
      """
    When I get skill "ns/live"
    Then the get response body contains "First version."
    When I overwrite the skill file at "ns/live.md" with body:
      """
      # Live

      Second version.
      """
    And  I get skill "ns/live"
    Then the get response body contains "Second version."

  Scenario: getting an unknown skill returns a not-found error
    When I get skill "no-such-skill-does-not-exist"
    Then the get fails with a message mentioning "not found"

  Scenario: get rejects a non-iii:// URI scheme
    When I get skill "https://example.com"
    Then the get fails with a message mentioning "iii://"

  # ── invalid id rejection ─────────────────────────────────────────────

  Scenario: a skill file with uppercase in its name is skipped from the listing
    Given a skill file at "ns/Bad-Name.md" with body:
      """
      # Bad

      Body.
      """
    When I list skills
    Then no listing entry has id "ns/Bad-Name"
    And  no listing entry has id "ns/bad-name"
