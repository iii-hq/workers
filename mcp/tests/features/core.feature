@engine @core
Feature: MCP core JSON-RPC methods

  Background:
    Given the mcp dispatcher is up

  Scenario: initialize negotiates protocol version 2025-06-18
    When I send the JSON-RPC request "initialize" with id 1
    Then the response advertises protocol version 2025-06-18
    And the response advertises the tools, resources, and prompts capabilities

  Scenario: ping returns an empty result
    When I send the JSON-RPC request "ping" with id 2
    Then the response result is empty

  Scenario: unknown method returns -32601
    When I send the JSON-RPC request "this/does/not/exist" with id 3
    Then the response is a JSON-RPC error with code -32601

  Scenario: notifications/initialized is acknowledged with HTTP 204
    When I send a JSON-RPC notification with no id
    Then the dispatcher returned a 204 with empty body for the notification
