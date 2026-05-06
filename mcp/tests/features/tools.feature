@engine @tools
Feature: tools/list advertises engine functions through MCP encoding
  `tools/list` enumerates `engine::functions::list`, drops anything
  whose function id matches a `hidden_prefixes` entry, and encodes
  remaining ids by replacing `::` with `__`. These scenarios pin the
  envelope shape and the hidden-prefix floor on a clean engine.

  Background:
    Given the iii engine is reachable

  Scenario: tools/list returns a tools array
    When I send a tools/list request
    Then the http response status is 200
    And  the response Content-Type starts with "text/event-stream"
    And  the JSON-RPC envelope has jsonrpc "2.0" and id 2
    And  result.tools is an array

  Scenario: hidden prefixes never leak through tools/list
    When I send a tools/list request
    Then result.tools is an array
    And  no advertised tool name starts with a hidden prefix
