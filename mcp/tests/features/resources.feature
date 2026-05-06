@engine @resources
Feature: resources/* delegates to skills with empty fallbacks
  The bridge forwards `resources/list`, `resources/read`, and
  `resources/templates/list` to the `skills` worker. When skills isn't
  registered (the bdd harness only registers `mcp::handler`), the
  bridge falls through to empty payloads so MCP clients still see a
  well-formed envelope. Pinning that fallback contract here.

  Background:
    Given the iii engine is reachable

  Scenario: resources/list returns a resources array
    When I send a resources/list request
    Then the http response status is 200
    And  the JSON-RPC envelope has jsonrpc "2.0" and id 10
    And  result.resources is an array

  Scenario: resources/read for an unknown URI returns an empty contents array
    When I send a resources/read request for uri "iii://does-not-exist"
    Then the http response status is 200
    And  result.contents is an array

  Scenario: resources/templates/list returns a resourceTemplates array
    When I send a resources/templates/list request
    Then the http response status is 200
    And  result.resourceTemplates is an array
