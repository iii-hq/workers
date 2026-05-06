@engine @skills_fetch
Feature: skills::fetch_skill and skill::fetch — batched iii:// resource fetch
  The fetch tool resolves one or more iii:// URIs through the same
  in-process resolver as `skills::resources-read`, wraps each result
  as `# {uri}\n\n{text}`, and joins sections with `\n\n---\n\n`.
  Both ids share one handler body: `skills::fetch_skill` is the
  internal id (hidden from MCP), `skill::fetch` is the public alias
  that MCP clients see.

  Background:
    Given the iii engine is reachable
    And   a fetch-skill seeded as "alpha" with body:
      """
      # alpha-skill
      Alpha body line.
      """
    And   a fetch-skill seeded as "beta" with body:
      """
      # beta-skill
      Beta body line.
      """

  # ── happy paths ───────────────────────────────────────────────────────

  Scenario: skills::fetch_skill with a single uri returns the body wrapped under the URI heading
    When I call skills::fetch_skill on seeds "alpha"
    Then the fetch call succeeds
    And  the fetch text starts with "# iii://" followed by the seeded "alpha" id
    And  the fetch text contains "Alpha body line."
    And  the fetch text has 1 section

  Scenario: skills::fetch_skill with a uris array concatenates sections separated by ---
    When I call skills::fetch_skill on seeds "alpha,beta"
    Then the fetch call succeeds
    And  the fetch text contains "Alpha body line."
    And  the fetch text contains "Beta body line."
    And  the fetch text has 2 sections

  Scenario: skill::fetch alias returns identical output to skills::fetch_skill
    When I call skill::fetch on seeds "alpha,beta"
    Then the fetch call succeeds
    And  the fetch text contains "Alpha body line."
    And  the fetch text contains "Beta body line."
    And  the fetch text has 2 sections

  Scenario: uris takes precedence over uri when both are provided
    When I call skills::fetch_skill with both seeds "beta" in uris and seed "alpha" in uri
    Then the fetch call succeeds
    And  the fetch text contains "Beta body line."
    And  the fetch text does not contain "Alpha body line."
    And  the fetch text has 1 section

  # ── validation ───────────────────────────────────────────────────────

  Scenario: missing uri and uris fields returns a validation error
    When I call skills::fetch_skill with no uri or uris
    Then the fetch call fails with a message mentioning "Provide uri"

  Scenario: a non-iii:// URI is rejected
    When I call skills::fetch_skill with the raw URI "https://example.com"
    Then the fetch call fails with a message mentioning "iii://"

  Scenario: a blank uri string is rejected
    When I call skills::fetch_skill with the raw URI "   "
    Then the fetch call fails with a message mentioning "Provide uri"

  Scenario: an unknown iii:// id surfaces a not-found error
    When I call skills::fetch_skill with the raw URI "iii://does-not-exist-xyz"
    Then the fetch call fails with a message mentioning "not found"

  # ── alias parity on the validation branch ────────────────────────────

  Scenario: skill::fetch surfaces the same validation error as skills::fetch_skill
    When I call skill::fetch with no uri or uris
    Then the fetch call fails with a message mentioning "Provide uri"
