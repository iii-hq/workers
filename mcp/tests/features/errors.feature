@engine @errors
Feature: malformed and unsupported requests return JSON-RPC errors
  Pins the bridge's JSON-RPC error envelopes for the four shapes the
  spec calls out: invalid request (-32600), method not found
  (-32601), invalid params (-32602). The bridge returns these with a
  200 HTTP status — the JSON-RPC error is in the SSE body, not the
  HTTP layer.

  Background:
    Given the iii engine is reachable

  Scenario: a JSON array body is rejected with -32600 (Invalid Request)
    When I send a JSON array body
    Then the http response status is 200
    And  the JSON-RPC error code is -32600
    And  the JSON-RPC error message contains "Batch"
    And  the JSON-RPC envelope has no result

  Scenario: a body missing the jsonrpc field is rejected with -32600
    When I send a body that is missing the jsonrpc field
    Then the JSON-RPC error code is -32600
    And  the JSON-RPC error message contains "Invalid"

  Scenario: an unknown method is rejected with -32601 (Method not found)
    When I send an unknown method "foo/bar"
    Then the JSON-RPC error code is -32601
    And  the JSON-RPC error message contains "foo/bar"

  Scenario: tools/call with a missing name is rejected with -32602 (Invalid params)
    When I send a tools/call with name ""
    Then the JSON-RPC error code is -32602
    And  the JSON-RPC error message contains "name"

  Scenario: tools/call resolving to a hidden prefix is rejected with -32602
    When I send a tools/call with name "engine__functions__list"
    Then the JSON-RPC error code is -32602
    And  the JSON-RPC error message contains "hidden"
