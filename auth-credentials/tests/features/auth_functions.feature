@pure @auth @auth_functions
Feature: auth::* credential functions
  Credential behavior should be reviewable from scenarios: stored credentials
  win over environment fallback, status never leaks token bytes, provider lists
  reveal names only, deletes are idempotent, OAuth payloads survive round trip,
  blank environment values are ignored, and invalid provider ids fail.

  Background:
    Given an empty auth credential store

  Scenario: stored credential wins over environment fallback
    Given environment variable "ANTHROPIC_API_KEY" is "sk-env-secret"
    When I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": { "type": "api_key", "key": "sk-stored-secret" }
      }
      """
    And I call auth::get_token with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth credential response has api key "sk-stored-secret"

  Scenario: set_token overwrites a rotated stored API key
    When I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": { "type": "api_key", "key": "sk-old-secret" }
      }
      """
    And I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": { "type": "api_key", "key": "sk-new-secret" }
      }
      """
    And I call auth::get_token with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth credential response has api key "sk-new-secret"
    And the auth response does not contain "sk-old-secret"

  Scenario: provider names are trimmed before storage and lookup
    When I call auth::set_token with payload:
      """
      {
        "provider": "  anthropic  ",
        "credential": { "type": "api_key", "key": "sk-trimmed" }
      }
      """
    And I call auth::get_token with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth credential response has api key "sk-trimmed"
    When I call auth::list_providers with payload:
      """
      {}
      """
    Then the auth provider list is "anthropic"

  Scenario: OAuth credentials preserve token metadata but status redacts it
    When I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": {
          "type": "oauth",
          "access_token": "oauth-access-secret",
          "refresh_token": "oauth-refresh-secret",
          "expires_at": 1893456000,
          "scopes": ["models:read", "messages:write"],
          "provider_extra": { "workspace": "prod" }
        }
      }
      """
    And I call auth::get_token with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth OAuth response has access token "oauth-access-secret"
    And the auth OAuth response has refresh token "oauth-refresh-secret"
    And the auth OAuth response has scopes "models:read,messages:write"
    When I call auth::status with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth status source is "stored"
    And the auth status label is "oauth"
    And the auth response does not contain "oauth-access-secret"
    And the auth response does not contain "oauth-refresh-secret"

  Scenario: status prefers stored credential and redacts stored and environment secrets
    Given environment variable "ANTHROPIC_API_KEY" is "sk-env-secret"
    When I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": { "type": "api_key", "key": "sk-stored-secret" }
      }
      """
    And I call auth::status with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth status source is "stored"
    And the auth status label starts with "api-key:sk-st"
    And the auth response does not contain "sk-stored-secret"
    And the auth response does not contain "sk-env-secret"

  Scenario: get_token falls back to the provider environment variable
    Given environment variable "OPENAI_API_KEY" is "sk-env-openai"
    When I call auth::get_token with payload:
      """
      { "provider": "openai" }
      """
    Then the auth credential response has api key "sk-env-openai"

  Scenario: status reports source without leaking the full credential
    Given environment variable "OPENAI_API_KEY" is "sk-env-openai-secret"
    When I call auth::status with payload:
      """
      { "provider": "openai" }
      """
    Then the auth status source is "environment"
    And the auth response does not contain "sk-env-openai-secret"

  Scenario: empty provider environment variable is ignored
    Given environment variable "OPENAI_API_KEY" is ""
    When I call auth::get_token with payload:
      """
      { "provider": "openai" }
      """
    Then the auth response is null
    When I call auth::status with payload:
      """
      { "provider": "openai" }
      """
    Then the auth status is unconfigured

  Scenario: unknown provider does not use unrelated environment fallback
    Given environment variable "OPENAI_API_KEY" is "sk-env-openai"
    When I call auth::get_token with payload:
      """
      { "provider": "unknown-provider" }
      """
    Then the auth response is null
    When I call auth::status with payload:
      """
      { "provider": "unknown-provider" }
      """
    Then the auth status is unconfigured

  Scenario: list_providers returns sorted names only
    When I call auth::set_token with payload:
      """
      {
        "provider": "openai",
        "credential": { "type": "api_key", "key": "sk-openai" }
      }
      """
    And I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": { "type": "api_key", "key": "sk-anthropic" }
      }
      """
    And I call auth::list_providers with payload:
      """
      {}
      """
    Then the auth provider list is "anthropic,openai"
    And the auth response does not contain "sk-openai"
    And the auth response does not contain "sk-anthropic"

  Scenario: delete_token is idempotent and removes the stored credential
    When I call auth::set_token with payload:
      """
      {
        "provider": "anthropic",
        "credential": { "type": "api_key", "key": "sk-delete-me" }
      }
      """
    And I call auth::delete_token with payload:
      """
      { "provider": "anthropic" }
      """
    And I call auth::delete_token with payload:
      """
      { "provider": "anthropic" }
      """
    And I call auth::get_token with payload:
      """
      { "provider": "anthropic" }
      """
    Then the auth response is null

  Scenario: delete_token reveals environment fallback after removing stored credential
    Given environment variable "OPENAI_API_KEY" is "sk-env-openai"
    When I call auth::set_token with payload:
      """
      {
        "provider": "openai",
        "credential": { "type": "api_key", "key": "sk-stored-openai" }
      }
      """
    And I call auth::delete_token with payload:
      """
      { "provider": "openai" }
      """
    And I call auth::get_token with payload:
      """
      { "provider": "openai" }
      """
    Then the auth credential response has api key "sk-env-openai"
    When I call auth::status with payload:
      """
      { "provider": "openai" }
      """
    Then the auth status source is "environment"
    And the auth response does not contain "sk-stored-openai"

  Scenario: blank provider ids fail before touching storage
    When I call auth::set_token with payload:
      """
      {
        "provider": " ",
        "credential": { "type": "api_key", "key": "sk-invalid" }
      }
      """
    Then the auth call fails with a message mentioning "provider must be non-empty"
