@engine @read
Feature: filesystem-backed reads (skills::list / skill::fetch)
  All read paths source from `skills_folder` on disk. Files arrive
  there via `skills::download` (or by direct editing in tests). Scans
  derive ids from the path relative to `skills_folder` with `.md`
  stripped and `prompts/` segments excluded. The `iii://` URI surface
  is reachable via `skill::fetch` (a non-`skills::*` alias of the
  internal resolver pipeline) — there are no MCP-shaped wrappers any
  more.

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

  # ── iii://{id} reads via skill::fetch ────────────────────────────────

  Scenario: iii://{id} returns the file body fresh
    Given a skill file at "ns/lookup.md" with body:
      """
      # Lookup

      Body content here.
      """
    When I read the URI "iii://ns/lookup"
    Then the fetched text contains "Body content here."

  Scenario: file changes between reads are reflected immediately
    Given a skill file at "ns/live.md" with body:
      """
      # Live

      First version.
      """
    When I read the URI "iii://ns/live"
    Then the fetched text contains "First version."
    When I overwrite the skill file at "ns/live.md" with body:
      """
      # Live

      Second version.
      """
    And  I read the URI "iii://ns/live"
    Then the fetched text contains "Second version."

  Scenario: reading an unknown skill returns a not-found error
    When I read the URI "iii://no-such-skill-does-not-exist"
    Then the read fails with a message mentioning "not found"

  # ── auto-rendered iii://skills index ────────────────────────────────

  Scenario: the iii://skills index lists each fs entry with title and description
    Given a skill file at "indexed.md" with body:
      """
      # Indexed skill

      First paragraph summary.
      """
    When I read the URI "iii://skills"
    Then the fetched text contains "# Skills"
    And  the fetched text contains "Indexed skill"
    And  the fetched text contains "First paragraph summary."

  Scenario: nested fs skills are indented in the index by depth
    Given a skill file at "tree.md" with body:
      """
      # Tree root

      Top.
      """
    And   a skill file at "tree/leaf.md" with body:
      """
      # Tree leaf

      Bottom.
      """
    When I read the URI "iii://skills"
    Then the index has entry "tree" indented by 0 spaces
    And  the index has entry "tree/leaf" indented by 2 spaces

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

  Scenario: a URI without the iii:// prefix is rejected
    When I read the URI "https://example.com"
    Then the read fails with a message mentioning "iii://"

  # ── skill::fetch composition ────────────────────────────────────────

  Scenario: skill::fetch concatenates bodies across depths
    Given a skill file at "fetched.md" with body:
      """
      # fetched

      ALPHA-BODY
      """
    And   a skill file at "fetched/sub.md" with body:
      """
      # fetched/sub

      BETA-BODY
      """
    And   a skill file at "fetched/sub/leaf.md" with body:
      """
      # fetched/sub/leaf

      GAMMA-BODY
      """
    When I fetch the URIs "iii://fetched, iii://fetched/sub, iii://fetched/sub/leaf"
    Then the fetched text contains "ALPHA-BODY"
    And  the fetched text contains "BETA-BODY"
    And  the fetched text contains "GAMMA-BODY"
    And  the fetched text has 3 sections joined by "---"
