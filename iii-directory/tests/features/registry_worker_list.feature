@engine @registry @registry_worker_list
Feature: directory::registry::workers::list (workers registry HTTP proxy)
  HTTP `GET {registry_base}/search?q=…&limit=…` proxied through to the
  workers registry. Responses are cached briefly per `(search, limit)`
  so the same lookup within `registry_cache_ttl_ms` doesn't re-hit
  HTTP. Row shape mirrors `directory::engine::workers::list` so callers
  learn one envelope.

  Background:
    Given the iii engine is reachable

  Scenario: workers::list forwards the search term and returns workers from the envelope
    Given a wiremock registry serving search "email" with body:
      """
      {
        "workers": [
          {
            "name": "resend",
            "latest_version": "1.2.3",
            "description": "Email worker",
            "repo": "https://github.com/iii-hq/resend",
            "author": { "name": "iii", "is_verified": true }
          },
          {
            "name": "mailgun",
            "latest_version": "0.4.0",
            "description": "Mailgun adapter"
          }
        ]
      }
      """
    When I trigger directory::registry::workers::list with payload:
      """
      {"search": "email"}
      """
    Then the directory::registry::workers::list call succeeds
    And  the registry worker-list response includes worker "resend"
    And  the registry worker-list response includes worker "mailgun"
    And  the registry worker-list response worker "resend" has version "1.2.3"

  Scenario: workers::list rejects an empty search
    When I trigger directory::registry::workers::list with payload:
      """
      {"search": "  "}
      """
    Then the directory::registry::workers::list call fails with a message mentioning "non-empty"

  Scenario: workers::list returns an empty list when the registry has no matches
    Given a wiremock registry serving search "nope" with body:
      """
      { "workers": [] }
      """
    When I trigger directory::registry::workers::list with payload:
      """
      {"search": "nope"}
      """
    Then the directory::registry::workers::list call succeeds
    And  the registry worker-list response is empty

  Scenario: registry HTTP error surfaces in the failure message
    Given a wiremock registry that returns 502 for search "broken"
    When I trigger directory::registry::workers::list with payload:
      """
      {"search": "broken"}
      """
    Then the directory::registry::workers::list call fails with a message mentioning "502"
