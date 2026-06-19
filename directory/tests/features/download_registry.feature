@engine @download @download_registry
Feature: directory::skills::download worker= source (workers registry HTTP)
  Pulls a worker's skills + prompts bundle from
  `{registry_url}/w/{worker}/skills?version=…` into
  `<skills_folder>/<worker>/`. Skills are written verbatim; prompts
  are written under `<worker>/prompts/<name>.md` with YAML frontmatter.
  The user-facing input accepts either `version:` (exact semver) or
  `tag:` (e.g. `latest`); both are forwarded as `?version=` per the
  OpenAPI contract.

  Background:
    Given the iii engine is reachable

  Scenario: downloads skills and prompts from the registry
    Given a wiremock registry serving worker "resend" at version "1.2.3" with body:
      """
      {
        "name": "resend",
        "version": "1.2.3",
        "skills": [
          { "path": "index.md", "content": "# resend\\nrouter body" },
          { "path": "emails/send-email.md", "content": "# send-email\\nleaf body" }
        ],
        "prompts": [
          {
            "name": "send-email",
            "description": "compose and send",
            "args_schema": { "type": "object" },
            "content": "Compose a friendly email."
          }
        ]
      }
      """
    When I trigger directory::skills::download with worker="resend" version="1.2.3"
    Then the directory::skills::download call succeeds
    And  the file "resend/index.md" in skills_folder contains "router body"
    And  the file "resend/emails/send-email.md" in skills_folder contains "leaf body"
    And  the file "resend/prompts/send-email.md" in skills_folder contains "Compose a friendly email."
    And  the file "resend/prompts/send-email.md" in skills_folder contains "description: compose and send"
    And  the download skills_written count is 2
    And  the download prompts_written count is 1

  Scenario: tag=latest forwards as a query parameter
    Given a wiremock registry serving worker "resend" at tag "latest" with body:
      """
      {
        "name": "resend",
        "version": "0.0.7",
        "skills": [
          { "path": "index.md", "content": "tagged" }
        ],
        "prompts": []
      }
      """
    When I trigger directory::skills::download with worker="resend" tag="latest"
    Then the directory::skills::download call succeeds
    And  the file "resend/index.md" in skills_folder contains "tagged"

  Scenario: registry HTTP error surfaces in the failure message
    Given a wiremock registry that returns 404 for worker "missing"
    When I trigger directory::skills::download with worker="missing" tag="latest"
    Then the directory::skills::download call fails with a message mentioning "404"

  Scenario: worker without version or tag defaults to tag=latest
    Given a wiremock registry serving worker "resend" at tag "latest" with body:
      """
      {
        "name": "resend",
        "version": "0.0.9",
        "skills": [
          { "path": "index.md", "content": "default-latest" }
        ],
        "prompts": []
      }
      """
    When I trigger directory::skills::download with worker="resend" alone
    Then the directory::skills::download call succeeds
    And  the file "resend/index.md" in skills_folder contains "default-latest"
