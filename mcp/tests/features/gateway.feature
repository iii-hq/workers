@engine @gateway
Feature: gateway tools, sessions and Origin over Streamable HTTP
  In gateway mode the server offers iii_call / iii_observe and friends,
  `initialize` opens a session returned as `Mcp-Session-Id`, observers
  belong to that session, and a browser Origin other than loopback is
  refused. The harness connects straight to the engine (no rbac-proxy).

  Background:
    Given the iii engine is reachable

  Scenario: tools/list offers the gateway tools first
    When I send a tools/list request
    Then the advertised tools start with the gateway set

  Scenario: iii_call reaches an engine function
    When I call iii_call with function "engine::workers::list"
    Then the tool result is not an error
    And  the tool result mentions "workers"

  Scenario: a foreign Origin is refused with 403
    When I send a ping request from Origin "https://evil.example"
    Then the http response status is 403

  Scenario: a loopback Origin is allowed
    When I send a ping request from Origin "http://localhost:5173"
    Then the http response status is 200

  Scenario: an unknown session is 404
    When I send a ping request with session "no-such-session"
    Then the http response status is 404

  Scenario: observe, trigger, read the event, then end the session
    Given I open an MCP session
    When I observe "state::set" for this scenario's scope
    And  I set a state key in this scenario's scope
    Then iii_events returns 1 event for this scenario's scope within 5 seconds
    When I end the session with DELETE
    Then the http response status is 200
    When I send a ping request with the ended session
    Then the http response status is 404
