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
    And  the listing entry "ns/labelled" has a null type

  Scenario: list rows prefer the frontmatter title and surface the frontmatter type
    Given a skill file at "ns/branded.md" with body:
      """
      ---
      title: Real title from frontmatter
      type: how-to
      ---
      # Body H1 ignored

      First paragraph summary.
      """
    When I list skills
    Then the listing entry "ns/branded" has title "Real title from frontmatter"
    And  the listing entry "ns/branded" has type "how-to"
    And  the listing entry "ns/branded" has description "First paragraph summary."

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
    And  the get response has a null type

  Scenario: directory::skills::get prefers the frontmatter title and exposes the frontmatter type
    Given a skill file at "ns/branded-get.md" with body:
      """
      ---
      title: Real title from frontmatter
      type: how-to
      ---
      # Body H1 ignored

      Body content here.
      """
    When I get skill "ns/branded-get"
    Then the get response has title "Real title from frontmatter"
    And  the get response has type "how-to"
    And  the get response body contains "Body H1 ignored"

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

  # ── directory::skills::index ─────────────────────────────────────────

  Scenario: directory::skills::index lists workers and filters out non-index skills
    Given a skill file at "team-a/index.md" with body:
      """
      ---
      title: team-a
      type: index
      ---
      # team-a

      Alpha team's worker. Owns alpha-only tooling.
      """
    And   a skill file at "team-b/index.md" with body:
      """
      ---
      title: team-b
      type: index
      ---
      # team-b

      Bravo team's worker. Handles all bravo workflows.
      """
    And   a skill file at "team-a/foo.md" with body:
      """
      ---
      type: how-to
      ---
      # Foo

      A how-to that must not appear in the worker index.
      """
    When I index skills
    Then the index response has at least 2 workers
    And  the index response body contains "# Skills index"
    And  the index response body contains "## team-a"
    And  the index response body contains "## team-b"
    And  the index response body contains "Alpha team's worker. Owns alpha-only tooling."
    And  the index response body contains "Bravo team's worker. Handles all bravo workflows."
    And  the index response body contains "Read [`team-a/index.md`](team-a/index.md) for the full worker reference."
    And  the index response body contains "Read [`team-b/index.md`](team-b/index.md) for the full worker reference."
    And  the index response body does not contain "iii://"
    And  the index response body does not contain "team-a/foo"
    And  the index response body does not contain "## Foo"

  # ── SKILLS.md alias (case-sensitive entry-point) ────────────────────

  Scenario: SKILLS.md at namespace root is aliased to <ns>/index
    Given a skill file at "ns-skills/SKILLS.md" with body:
      """
      # ns-skills entry-point

      Body served via the SKILLS.md alias.
      """
    When I list skills
    Then the listing has an entry with id "ns-skills/index"
    And  no listing entry has id "ns-skills/SKILLS"

  Scenario: nested SKILLS.md is aliased to <ns>/<sub>/index
    Given a skill file at "ns-nested/section/SKILLS.md" with body:
      """
      # nested entry-point

      Body for nested SKILLS.md.
      """
    When I list skills
    Then the listing has an entry with id "ns-nested/section/index"

  # ── directory::skills::get accepts the file-path forms ──────────────

  Scenario: directory::skills::get accepts the <id>.md file-path form
    Given a skill file at "ns-path/lookup.md" with body:
      """
      # Lookup via path

      Body for the path-form lookup.
      """
    When I get skill "ns-path/lookup.md"
    Then the get response has id "ns-path/lookup"
    And  the get response body contains "Body for the path-form lookup."

  Scenario: directory::skills::get accepts the SKILLS.md filename
    Given a skill file at "ns-skills-get/SKILLS.md" with body:
      """
      # ns-skills-get entry-point

      Body served via SKILLS.md lookup.
      """
    When I get skill "ns-skills-get/SKILLS.md"
    Then the get response has id "ns-skills-get/index"
    And  the get response body contains "Body served via SKILLS.md lookup."

  Scenario: directory::skills::get accepts the iii:// + .md combined form
    Given a skill file at "ns-combo/lookup.md" with body:
      """
      # Combo

      Body for combined-form lookup.
      """
    When I get skill "iii://ns-combo/lookup.md"
    Then the get response has id "ns-combo/lookup"
    And  the get response body contains "Body for combined-form lookup."
