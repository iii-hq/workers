@engine @resources
Feature: MCP resources/* delegation to skills::resources-*

  Scenario: resources/list delegates to skills::resources-list
    When I list resources
    Then the resource listing includes the iii://skills index

  Scenario: resources/read delegates to skills::resources-read
    When I read the resource "iii://demo"
    Then the resource read mime type is "text/markdown"
    And the resource read text mentions "iii://demo"

  Scenario: resources/read without params returns -32602
    When I send resources/read with no params
    Then the response is a JSON-RPC error with code -32602 for resources

  Scenario: resources/templates/list delegates to skills::resources-templates
    When I list resource templates
    Then the templates listing has the skill and skill-section URIs
