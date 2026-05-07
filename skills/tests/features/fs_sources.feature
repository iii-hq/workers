@engine @fs_sources
Feature: filesystem-backed skills and prompts (skills: / prompts: globs)
  Files matched by the `skills:` and `prompts:` glob patterns in the
  worker config become registry entries whose body is read fresh from
  disk on every resolve. The file system is the single source of
  truth — nothing is mirrored to iii-state. The auto-rendered
  `iii://skills` index gets a separate `## Custom skills` section.

  Background:
    Given the iii engine is reachable

  # ── flat folder of skills ────────────────────────────────────────────

  Scenario: a flat folder of fs skills appears in skills::list with origin=fs
    Given a fs skill file at "alpha.md" with body:
      """
      # Alpha

      The alpha skill body.
      """
    And   a fs skill file at "beta.md" with body:
      """
      # Beta

      The beta skill body.
      """
    When I list skills
    Then the listing has a fs entry with id "alpha" and origin "fs"
    And  the listing has a fs entry with id "beta" and origin "fs"

  # ── nested directory hierarchy ───────────────────────────────────────

  Scenario: nested folders derive slashed ids
    Given a fs skill file at "team-a/playbook.md" with body:
      """
      # Team A playbook

      Top-level body.
      """
    And   a fs skill file at "team-a/meetings/standup.md" with body:
      """
      # Standup

      Nested body.
      """
    When I list skills
    Then the listing has a fs entry with id "team-a/playbook" and origin "fs"
    And  the listing has a fs entry with id "team-a/meetings/standup" and origin "fs"

  # ── iii://{id} reads ─────────────────────────────────────────────────

  Scenario: iii://{id} returns the file body fresh
    Given a fs skill file at "lookup.md" with body:
      """
      # Lookup

      Body content here.
      """
    When I read the URI "iii://lookup"
    Then the contents mime type is "text/markdown"
    And  the contents text contains "Body content here."

  Scenario: file changes between reads are reflected immediately
    Given a fs skill file at "live.md" with body:
      """
      # Live

      First version.
      """
    When I read the URI "iii://live"
    Then the contents text contains "First version."
    When I overwrite the fs skill file at "live.md" with body:
      """
      # Live

      Second version.
      """
    And  I read the URI "iii://live"
    Then the contents text contains "Second version."

  # ── index rendering ──────────────────────────────────────────────────

  Scenario: the iii://skills index has a Custom skills H2 section
    Given a fs skill file at "indexed.md" with body:
      """
      # Indexed skill

      First paragraph summary.
      """
    When I read iii://skills
    Then the contents text contains "## Custom skills"
    And  the contents text contains "Indexed skill"
    And  the contents text contains "First paragraph summary."

  Scenario: nested fs skills are indented in the index by depth
    Given a fs skill file at "tree.md" with body:
      """
      # Tree root

      Top.
      """
    And   a fs skill file at "tree/leaf.md" with body:
      """
      # Tree leaf

      Bottom.
      """
    When I read iii://skills
    Then the index has a fs entry "tree" indented by 0 spaces
    And  the index has a fs entry "tree/leaf" indented by 2 spaces

  # ── collisions ───────────────────────────────────────────────────────

  Scenario: a fs id colliding with a state-backed id is shadowed
    Given a fs skill file at "dual.md" with body:
      """
      # Dual

      File version.
      """
    When I register a skill with id "dual" and body "# Dual\n\nState version.\n"
    Then the skills::register call succeeds
    When I read the URI "iii://dual"
    Then the contents text contains "State version."
    And  the contents text does not contain "File version."

  # ── invalid id rejection ─────────────────────────────────────────────

  Scenario: a fs skill file with uppercase in its name is skipped
    Given a fs skill file at "Bad-Name.md" with body:
      """
      # Bad

      Body.
      """
    When I list skills
    Then no listing entry has id "Bad-Name"
    And  no listing entry has id "bad-name"

  # ── prompts ─────────────────────────────────────────────────────────

  Scenario: a fs prompt with frontmatter appears in mcp-list
    Given a fs prompt file at "open-pr.md" with content:
      """
      ---
      name: open-pr
      description: Open a GitHub pull request.
      ---
      Compose a PR body that follows the repository template.
      """
    When I call prompts::mcp-list
    Then the mcp prompt list includes "open-pr" with description "Open a GitHub pull request."

  Scenario: prompts/get for a fs prompt returns the body as a single user-text message
    Given a fs prompt file at "static.md" with content:
      """
      ---
      description: Static prompt.
      ---
      The body content of the prompt.
      """
    When I call prompts::mcp-get for "static"
    Then the mcp prompt response description is "Static prompt."
    And  the mcp prompt response has a single user-text message
    And  the mcp prompt user-text message contains "The body content of the prompt."

  Scenario: a fs prompt without frontmatter is skipped
    Given a fs prompt file at "no-fm.md" with content:
      """
      # heading
      no frontmatter
      """
    When I call prompts::mcp-list
    Then the mcp prompt list does not include "no-fm"
