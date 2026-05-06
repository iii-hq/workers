@engine @initialize
Feature: initialize negotiates a fixed protocol version
  The bridge advertises protocol `2025-06-18` regardless of what the
  client claims, exposes tools/resources/prompts capabilities, and
  identifies itself as the `mcp` worker. Drives the handshake over
  HTTP so the SSE envelope and Content-Type are pinned too.

  Background:
    Given the iii engine is reachable

  Scenario: initialize round-trips through the SSE bridge
    When I send an initialize request
    Then the http response status is 200
    And  the response Content-Type starts with "text/event-stream"
    And  the JSON-RPC envelope has jsonrpc "2.0" and id 1
    And  result.protocolVersion is "2025-06-18"
    And  result.capabilities.tools is an object
    And  result.capabilities.resources is an object
    And  result.capabilities.prompts is an object
    And  result.serverInfo.name is "mcp"
    And  result.serverInfo.version matches the crate version

  Scenario: initialize ignores the client's claimed protocolVersion
    When I send an initialize request claiming protocolVersion "1999-01-01"
    Then result.protocolVersion is "2025-06-18"

  Scenario: ping returns an empty result envelope
    When I send a ping request
    Then the http response status is 200
    And  result is an empty object
