@engine @registry @registry_worker_list
Feature: directory::registry::workers::list (workers registry HTTP proxy)
  HTTP `GET {registry_base}/w?search=…&cursor=…` proxied through to the
  workers registry. Page size is server-authored; clients pass back
  `pagination.next_cursor` to fetch the next page. Responses are cached
  briefly per `(search, cursor)` so the same lookup within
  `registry_cache_ttl_ms` doesn't re-hit HTTP. The shared core fields
  (`name`, `description`, `version`) line up with the engine's
  `engine::workers::list` so callers learn one envelope.

  Background:
    Given the iii engine is reachable

  Scenario: workers::list forwards the search term and returns the paginated envelope
    Given a wiremock registry serving search "email" with body:
      """
      {
        "workers": [
          {
            "name": "resend",
            "description": "Email worker",
            "type": "binary",
            "version": "1.2.3",
            "repo": "https://github.com/iii-hq/resend",
            "config": {},
            "supported_targets": ["x86_64-unknown-linux-gnu"],
            "total_downloads": 42,
            "dependencies": [],
            "author": { "name": "iii", "pfp": null, "verified": true }
          },
          {
            "name": "mailgun",
            "description": "Mailgun adapter",
            "type": "binary",
            "version": "0.4.0",
            "repo": "https://github.com/iii-hq/mailgun",
            "config": {},
            "supported_targets": [],
            "total_downloads": 7,
            "dependencies": [],
            "author": null
          }
        ],
        "pagination": {
          "next_cursor": "next-page",
          "has_more": true,
          "page_size": 20
        }
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
    And  the registry worker-list pagination has_more is true
    And  the registry worker-list pagination next_cursor is "next-page"

  Scenario: workers::list returns an empty list when the registry has no matches
    Given a wiremock registry serving search "nope" with body:
      """
      {
        "workers": [],
        "pagination": {
          "next_cursor": null,
          "has_more": false,
          "page_size": 20
        }
      }
      """
    When I trigger directory::registry::workers::list with payload:
      """
      {"search": "nope"}
      """
    Then the directory::registry::workers::list call succeeds
    And  the registry worker-list response is empty
    And  the registry worker-list pagination has_more is false
    And  the registry worker-list pagination next_cursor is null

  Scenario: registry HTTP error surfaces in the failure message
    Given a wiremock registry that returns 502 for search "broken"
    When I trigger directory::registry::workers::list with payload:
      """
      {"search": "broken"}
      """
    Then the directory::registry::workers::list call fails with a message mentioning "502"
