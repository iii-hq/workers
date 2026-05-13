@engine @registry @registry_worker_info
Feature: directory::registry::workers::info (workers registry HTTP proxy)
  HTTP `GET {registry_base}/w/{name}?version=…|tag=…` proxied to the
  workers registry. The flat publish payload is decoded into a
  `{ worker: { name, description, version, repo, author }, readme,
  api_reference: { functions, triggers }, skills_tree }` envelope —
  the `worker` field has the same shape as
  `directory::engine::workers::info.worker` so callers can switch
  between local + registry surfaces with one parser.

  Background:
    Given the iii engine is reachable

  Scenario: workers::info returns the full publish envelope at a tag
    Given a wiremock registry serving worker info "resend" at tag "latest" with body:
      """
      {
        "name": "resend",
        "version": "1.2.3",
        "description": "Email worker",
        "repo": "https://github.com/iii-hq/resend",
        "readme": "# resend\n\nDocs body.",
        "author": { "name": "iii", "is_verified": true },
        "functions": [
          {
            "name": "send",
            "description": "Send an email.",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
          }
        ],
        "triggers": [
          {
            "name": "on-bounce",
            "description": "Fires on a bounce."
          }
        ],
        "skills_tree": {
          "skills": [{ "path": "index.md" }],
          "prompts": [{ "name": "send-email", "description": "Compose." }]
        }
      }
      """
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "resend", "tag": "latest"}
      """
    Then the directory::registry::workers::info call succeeds
    And  the registry worker-info worker name is "resend"
    And  the registry worker-info worker version is "1.2.3"
    And  the registry worker-info worker description is "Email worker"
    And  the registry worker-info response has a non-empty readme
    And  the registry worker-info api_reference functions count is 1
    And  the registry worker-info api_reference triggers count is 1
    And  the registry worker-info skills_tree skills count is 1

  Scenario: workers::info defaults to tag latest when neither version nor tag is given
    Given a wiremock registry serving worker info "resend" at tag "latest" with body:
      """
      {
        "name": "resend",
        "version": "1.2.3",
        "functions": [],
        "triggers": [],
        "skills_tree": {"skills": [], "prompts": []}
      }
      """
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "resend"}
      """
    Then the directory::registry::workers::info call succeeds
    And  the registry worker-info worker name is "resend"

  Scenario: workers::info rejects both version and tag
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "resend", "version": "1.2.3", "tag": "latest"}
      """
    Then the directory::registry::workers::info call fails with a message mentioning "either version OR tag"

  Scenario: workers::info rejects an empty name
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "  "}
      """
    Then the directory::registry::workers::info call fails with a message mentioning "non-empty"

  Scenario: workers::info HTTP 404 surfaces in the failure message
    Given a wiremock registry that returns 404 for worker info "missing"
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "missing", "tag": "latest"}
      """
    Then the directory::registry::workers::info call fails with a message mentioning "404"

  Scenario: workers::info caches identical lookups within the TTL window
    Given a wiremock registry serving worker info "cached" at tag "latest" with body:
      """
      {
        "name": "cached",
        "version": "1.0.0",
        "functions": [],
        "triggers": [],
        "skills_tree": {"skills": [], "prompts": []}
      }
      """
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "cached", "tag": "latest"}
      """
    Then the directory::registry::workers::info call succeeds
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "cached", "tag": "latest"}
      """
    Then the directory::registry::workers::info call succeeds
    And  the wiremock registry received exactly 1 request to "/w/cached"
