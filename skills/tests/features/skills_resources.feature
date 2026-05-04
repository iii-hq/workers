@engine @skills_resources
Feature: iii:// resource resolver (skills::resources-list / read / templates)
  The `skills::resources-*` internal RPC serves the `iii://skills`
  index, registered `iii://{id}` bodies, and `iii://{id}/{fn}` sections
  that delegate to a sub-skill function. Normalization turns strings,
  `{content}` objects, and JSON into the contents envelope shape the
  `mcp` worker forwards to MCP `resources/read`.

  Background:
    Given the iii engine is reachable

  # ── resources-list ────────────────────────────────────────────────────

  Scenario: resources-list exposes the iii://skills index and every registered skill
    Given a skill with id "listed" and body:
      """
      # Listed

      This skill shows up in the index.
      """
    When I call skills::resources-list
    Then the resource list includes iii://skills
    And  the resource list includes the seeded skill URI

  Scenario: resources-templates returns the two URI templates
    When I call skills::resources-templates
    Then the templates listing contains the skill and skill-section templates

  # ── reading iii:// URIs ───────────────────────────────────────────────

  Scenario: iii://skills renders a markdown index with title and description
    Given a skill with id "indexed" and body:
      """
      # Indexed skill

      First paragraph summary.

      Ignored second paragraph.
      """
    When I read iii://skills
    Then the contents mime type is "text/markdown"
    And  the contents text contains "# Skills"
    And  the contents text contains "Indexed skill"
    And  the contents text contains "First paragraph summary."

  Scenario: iii://{id} returns the registered body as text/markdown
    Given a skill with id "body" and body:
      """
      # Body skill

      The full body goes here.
      """
    When I read the seeded skill URI
    Then the contents mime type is "text/markdown"
    And  the contents text contains "The full body goes here."

  # ── section shapes ────────────────────────────────────────────────────

  Scenario: iii://{skill}/{fn} returning a markdown string is served as text/markdown
    Given a skill with id "sec-str" and body:
      """
      # sec-str
      """
    And   a sub-skill function that returns a markdown string
    When I read the section URI with skill id "sec-str"
    Then the contents mime type is "text/markdown"
    And  the contents text contains "str-section"

  Scenario: iii://{skill}/{fn} returning {content} is served as text/markdown
    Given a skill with id "sec-obj" and body:
      """
      # sec-obj
      """
    And   a sub-skill function that returns a {content} object
    When I read the section URI with skill id "sec-obj"
    Then the contents mime type is "text/markdown"
    And  the contents text contains "wrapped"

  Scenario: iii://{skill}/{fn} returning arbitrary JSON falls back to application/json
    Given a skill with id "sec-json" and body:
      """
      # sec-json
      """
    And   a sub-skill function that returns an arbitrary JSON object
    When I read the section URI with skill id "sec-json"
    Then the contents mime type is "application/json"
    And  the contents text contains "count"

  # ── recursion guard ──────────────────────────────────────────────────

  Scenario: the section resolver refuses state:: as a function id
    When I read the URI "iii://anything/state::set"
    Then the read fails with a message mentioning "internal namespace"

  Scenario: the section resolver refuses mcp:: as a function id
    When I read the URI "iii://x/mcp::handler"
    Then the read fails with a message mentioning "internal namespace"

  Scenario: the section resolver refuses skills:: to block tunnelling into the admin API
    When I read the URI "iii://x/skills::register"
    Then the read fails with a message mentioning "internal namespace"

  Scenario: the section resolver refuses prompts:: as a function id
    When I read the URI "iii://x/prompts::register"
    Then the read fails with a message mentioning "internal namespace"

  # ── URI validation ───────────────────────────────────────────────────

  Scenario: a URI without the iii:// prefix is rejected
    When I read the URI "https://example.com"
    Then the read fails with a message mentioning "iii://"

  Scenario: a URI with more than two path segments is rejected
    When I read the URI "iii://a/b/c"
    Then the read fails with a message mentioning "more than one path segment"

  Scenario: reading an unknown skill returns a not-found error
    When I read the URI "iii://no-such-skill-does-not-exist"
    Then the read fails with a message mentioning "not found"
