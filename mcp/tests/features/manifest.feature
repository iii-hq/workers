@pure @manifest
Feature: --manifest emits the registry payload
  The iii registry pipeline reads `mcp --manifest` on publish; the
  default config it embeds is what every fresh deployment starts
  with. Subprocess-driven; no engine needed.

  Scenario: --manifest emits valid JSON the registry pipeline can read
    When I run `mcp --manifest`
    Then the exit status is success
    And  stdout is valid JSON
    And  the manifest field "name" is "mcp"
    And  the manifest field "version" matches the crate version

  Scenario: the manifest carries the canonical default api_path
    When I run `mcp --manifest`
    Then the manifest's default api_path is "/mcp"

  Scenario: the manifest disables require_expose by default
    When I run `mcp --manifest`
    Then the manifest's default require_expose is false

  Scenario: the manifest's hidden_prefixes covers the engine + bridge floors
    When I run `mcp --manifest`
    Then the manifest's default hidden_prefixes contains "engine::"
    And  the manifest's default hidden_prefixes contains "state::"
    And  the manifest's default hidden_prefixes contains "stream::"
    And  the manifest's default hidden_prefixes contains "iii."
    And  the manifest's default hidden_prefixes contains "iii::"
    And  the manifest's default hidden_prefixes contains "mcp::"
    And  the manifest's default hidden_prefixes contains "a2a::"
    And  the manifest's default hidden_prefixes contains "skills::"
    And  the manifest's default hidden_prefixes contains "prompts::"

  Scenario: the manifest declares at least one supported target
    When I run `mcp --manifest`
    Then the manifest's supported_targets is non-empty

  Scenario: --help exits cleanly and lists the bridge flags
    When I run `mcp --help`
    Then the exit status is success
    And  stdout mentions "--manifest"
    And  stdout mentions "--config"
    And  stdout mentions "--url"
