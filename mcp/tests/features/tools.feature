@engine @tools
Feature: MCP tools/list and tools/call

  Background:
    Given the bdd::echo and bdd::boom fixtures are registered

  Scenario: tools/list exposes user functions and hides infra namespaces
    When I list tools
    Then the tool listing includes bdd__echo with its inputSchema
    And the tool listing excludes hidden namespaces

  Scenario: tools/call dispatches to the iii function
    When I call the tool "bdd__echo" with arguments {msg=hi}
    Then the tool response is not marked isError
    And the tool response text contains "hi"

  Scenario: tools/call surfaces handler failures as isError
    When I call the tool "bdd__boom" with arguments {}
    Then the tool response is marked isError
    And the tool response text contains "kapow"

  Scenario: tools/call refuses hidden namespaces with a tool error
    When I call the tool "state__get" with arguments {scope=x,key=y}
    Then the tool response is marked isError
    And the tool response text contains "internal namespace"

  Scenario: tools/call without params returns -32602
    When I call tools/call with no params
    Then the response is a JSON-RPC error with code -32602
