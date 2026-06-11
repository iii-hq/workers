@pure
Feature: --manifest registry contract

  The registry publish pipeline runs `session-manager --manifest` and
  expects fast, side-effect-free JSON on stdout with five required
  fields. A malformed manifest breaks `POST /publish` for every release.

  # Prevents: shipping a binary whose --manifest output the registry rejects.
  Scenario: the manifest subcommand emits every required field
    When I run the worker binary with --manifest
    Then the manifest has every required registry field
    And the response field "name" is "session-manager"
    And the response field "default_config.backend" is "fs"
    And the response field "default_config.backend_config.data_dir" is "~/.iii/data/session-manager"
    And the response field "default_config.default_list_limit" is 50
    And the response field "default_config.max_list_limit" is 500
