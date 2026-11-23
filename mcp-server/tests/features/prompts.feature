@engine @prompts
Feature: the server offers no MCP prompts
  No prompts capability is advertised, so prompts methods are unknown.
  Agent profiles are loaded with iii_get_agent and listed with iii_call
  (directory::agents::list).

  Background:
    Given the iii engine is reachable

  Scenario: prompts/list is method not found
    When I send a prompts/list request
    Then the JSON-RPC error code is -32601

  Scenario: prompts/get is method not found
    When I send a prompts/get request for name "does-not-exist"
    Then the JSON-RPC error code is -32601
