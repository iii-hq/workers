@pure @auth @auth_skill_bundle
Feature: auth-credentials skill bundle
  The bundled markdown is an agent-facing contract. It should follow the
  worker skill bundle format: one index skill, namespace-mirrored how-to
  paths, function_id frontmatter, ordered sections, parseable JSON examples,
  and explicit side effects for write paths.

  Scenario: index skill identifies the worker and links every auth how-to
    Then the auth skill index has type "index" and title "auth-credentials"
    And the auth skill index links to every auth how-to

  Scenario: auth how-to files map paths to function ids
    Then every auth how-to path mirrors its function id
    And every auth how-to has required sections in order

  Scenario: auth how-to examples and side effects are reviewable
    Then every auth how-to JSON example parses
    And auth write how-tos document side effects
