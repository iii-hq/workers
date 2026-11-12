@engine @prompts
Feature: the server offers no MCP prompts
  Agent profiles are reached through the gateway tools (iii_list_agents,
  iii_get_agent) and the iii Claude plugin's /iii:* commands, so the
  server advertises no prompts capability. prompts/list stays answerable
  with an empty list for clients that ask anyway; prompts/get is invalid.

  Background:
    Given the iii engine is reachable

  Scenario: prompts/list returns an empty prompts array
    When I send a prompts/list request
    Then the http response status is 200
    And  the JSON-RPC envelope has jsonrpc "2.0" and id 20
    And  result.prompts is an array

  Scenario: prompts/get is invalid params
    When I send a prompts/get request for name "does-not-exist"
    Then the JSON-RPC error code is -32602
