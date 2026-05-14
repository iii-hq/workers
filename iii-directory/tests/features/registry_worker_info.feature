@engine @registry @registry_worker_info
Feature: directory::registry::workers::info (workers registry HTTP proxy)
  Two parallel registry calls back the response:
    * `GET {base}/w/{name}?version=…` returns `{ worker: WorkerDetail }`
      whose `worker` field carries name / description / version / repo /
      author / type / config / supported_targets / total_downloads /
      dependencies plus readme / functions / triggers.
    * `GET {base}/w/{name}/skills?version=…` returns the skills + prompts
      tree merged into the response as `skills_tree`.
  The OpenAPI uses `?version=` for both endpoints; tags (`latest`) and
  exact semvers share that wire param. The user-facing input still
  accepts `tag:` for ergonomics — the worker rewrites it to `?version=`.
  The `worker` field shares its core fields (`name`, `description`,
  `version`) with `directory::engine::workers::info.worker`.

  Background:
    Given the iii engine is reachable

  Scenario: workers::info merges the detail envelope and skills tree at a tag
    Given a wiremock registry serving worker info "resend" at tag "latest" with body:
      """
      {
        "worker": {
          "name": "resend",
          "description": "Email worker",
          "type": "binary",
          "version": "1.2.3",
          "repo": "https://github.com/iii-hq/resend",
          "config": {},
          "supported_targets": ["x86_64-unknown-linux-gnu"],
          "total_downloads": 4242,
          "dependencies": [],
          "author": { "name": "iii", "pfp": null, "verified": true },
          "readme": "# resend\n\nDocs body.",
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
              "description": "Fires on a bounce.",
              "invocation_schema": { "type": "object" },
              "return_schema": { "type": "object" }
            }
          ]
        }
      }
      """
    And a wiremock registry serving worker skills "resend" at version "latest" with body:
      """
      {
        "name": "resend",
        "version": "1.2.3",
        "skills": [{ "path": "index.md", "content": "# resend" }],
        "prompts": [
          {
            "name": "send-email",
            "description": "Compose.",
            "args_schema": { "type": "object" },
            "content": "Compose body."
          }
        ]
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
        "worker": {
          "name": "resend",
          "description": "Email worker",
          "type": "binary",
          "version": "1.2.3",
          "repo": "https://github.com/iii-hq/resend",
          "config": {},
          "supported_targets": [],
          "total_downloads": 0,
          "dependencies": [],
          "author": null,
          "readme": "",
          "functions": [],
          "triggers": []
        }
      }
      """
    And a wiremock registry serving worker skills "resend" at version "latest" with body:
      """
      { "name": "resend", "version": "1.2.3", "skills": [], "prompts": [] }
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
    And a wiremock registry that returns 404 for worker skills "missing"
    When I trigger directory::registry::workers::info with payload:
      """
      {"name": "missing", "tag": "latest"}
      """
    Then the directory::registry::workers::info call fails with a message mentioning "404"

  Scenario: workers::info caches identical lookups within the TTL window
    Given a wiremock registry serving worker info "cached" at tag "latest" with body:
      """
      {
        "worker": {
          "name": "cached",
          "description": null,
          "type": "binary",
          "version": "1.0.0",
          "repo": null,
          "config": {},
          "supported_targets": [],
          "total_downloads": 0,
          "dependencies": [],
          "author": null,
          "readme": "",
          "functions": [],
          "triggers": []
        }
      }
      """
    And a wiremock registry serving worker skills "cached" at version "latest" with body:
      """
      { "name": "cached", "version": "1.0.0", "skills": [], "prompts": [] }
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
    And  the wiremock registry received exactly 1 request to "/w/cached/skills"
