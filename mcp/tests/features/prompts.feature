@engine @prompts
Feature: MCP prompts/* delegation to prompts::mcp-*

  Scenario: prompts/list delegates to prompts::mcp-list
    When I list prompts
    Then the prompts listing includes "demo-greet" with a required argument

  Scenario: prompts/get delegates to prompts::mcp-get
    When I get the prompt "demo-greet" with arguments to=alice
    Then the prompt response has a description and a single user message
    And the prompt user message contains "Hello, alice!"

  Scenario: prompts/get without params returns -32602
    When I send prompts/get with no params
    Then the response is a JSON-RPC error with code -32602 for prompts
