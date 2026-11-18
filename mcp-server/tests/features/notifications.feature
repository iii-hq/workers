@engine @notifications
Feature: notifications return 202 Accepted with no body
  MCP Streamable HTTP §"server response": notification-only requests
  (no `id` field) MUST return 202. The bridge accepts both the
  spec'd `notifications/initialized` and any unknown notification.

  Background:
    Given the iii engine is reachable

  Scenario: notifications/initialized returns 202
    When I send a notifications/initialized notification
    Then the http response status is 202

  Scenario: an unknown notification still returns 202
    When I send an unknown notification with method "notifications/something-novel"
    Then the http response status is 202
