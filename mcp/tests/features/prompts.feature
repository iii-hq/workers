@engine @prompts
Feature: prompts/* delegates to skills with empty fallbacks
  `prompts/list` and `prompts/get` route to the skills worker through
  `prompts::mcp-list` / `prompts::mcp-get`. When that worker isn't
  registered the bridge returns empty `prompts` / `messages` arrays
  so MCP clients still see a well-formed envelope.

  Background:
    Given the iii engine is reachable

  Scenario: prompts/list returns a prompts array
    When I send a prompts/list request
    Then the http response status is 200
    And  the JSON-RPC envelope has jsonrpc "2.0" and id 20
    And  result.prompts is an array

  Scenario: prompts/get for an unknown name returns an empty messages array
    When I send a prompts/get request for name "does-not-exist"
    Then the http response status is 200
    And  result.messages is an array
