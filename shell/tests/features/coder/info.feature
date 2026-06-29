@direct @info
Feature: coder info contract

  Scenario: reports every jail root and configured cap
    Given a jailed code surface
    When I call coder::info with payload:
      """
      {}
      """
    Then the call succeeded
    And the info exposes 2 base paths
    And the info includes non-accessible glob "**/.env"
    And the info field "max_read_bytes" equals 512
    And the info field "max_write_bytes" equals 512
    And the info field "batch_read_budget_bytes" equals 160
    And the info field "list_max_page_size" equals 4
