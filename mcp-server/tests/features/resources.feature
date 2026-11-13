@engine @resources
Feature: resources/* serve skills from iii-directory
  Skills are resources at `iii://<id>`, read from the `iii-directory`
  worker (`directory::skills::list` / `directory::skills::get`). An
  unknown skill, a foreign URI, or a missing directory all give empty
  but well-formed envelopes.

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
