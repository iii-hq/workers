@engine @mcp_bridge
Feature: skills::resources-* and prompts::mcp-* bridge to the mcp worker
  The `mcp` worker serves MCP `resources/*` and `prompts/*` by calling
  the internal RPC this worker exposes. These scenarios lock in the
  exact envelope shape the `mcp` dispatcher expects so a renamed
  field or missing key is caught immediately.

  Background:
    Given the iii engine is reachable
    And   a skill and a prompt are registered for the bridge test

  Scenario: skills::resources-list returns the MCP resources/list envelope
    When I call skills::resources-list through the bridge
    Then the bridged resources envelope has the iii://skills index
    And  the bridged resources envelope includes the seeded skill uri

  Scenario: skills::resources-read round-trips a registered skill body
    When I call skills::resources-read on the seeded skill uri
    Then the bridged read contents mime is text/markdown
    And  the bridged read contents text includes the skill body

  Scenario: prompts::mcp-list returns the MCP-shaped arguments array
    When I call prompts::mcp-list through the bridge
    Then the bridged prompts listing includes the seeded prompt with a required argument

  Scenario: prompts::mcp-get dispatches the handler and returns the MCP messages shape
    When I call prompts::mcp-get through the bridge with arguments to=alice@example.com
    Then the bridged prompts::mcp-get returns a single user message

  # The fetch tool is registered twice: `skills::fetch_skill` (canonical,
  # hidden by the MCP hard floor) and `skill::fetch` (public alias).
  # This scenario locks in that the alias namespace stays non-hidden so
  # the mcp worker continues to surface it in `tools/list`.
  Scenario: skill::fetch is registered on a non-hidden namespace so the MCP bridge surfaces it
    When I list registered functions through engine::functions::list
    Then the listing includes a function "skill::fetch"
    And  the function "skill::fetch" has a non-empty description
    And  the function "skill::fetch" is in a non-hidden namespace
    And  the listing includes a function "skills::fetch_skill"
    And  the function "skills::fetch_skill" is in the always-hidden namespace floor
