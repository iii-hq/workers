@direct @search
Feature: coder search adversarial behavior

  Scenario: content search returns line numbers and bounded context
    Given a jailed code surface
    And a file at "src/lib.rs" with content:
      """
      fn before() {}
      fn target() {}
      fn after() {}
      """
    When I call coder::search with payload:
      """
      {"query":"target","path":"src","search_paths":false,"context_lines_before":1,"context_lines_after":1}
      """
    Then the search has a content match for "src/lib.rs" at line 2
    And the search truncated flag is false

  Scenario: path-only search does not inspect file contents
    Given a jailed code surface
    And a file at "src/searchable-name.txt" with content:
      """
      no marker here
      """
    When I call coder::search with payload:
      """
      {"query":"searchable","path":".","search_content":false,"search_paths":true}
      """
    Then the search has a path match for "src/searchable-name.txt"

  Scenario: protected files are omitted from search results
    Given a jailed code surface
    And a file at ".env" with content:
      """
      TOKEN=super-secret
      """
    When I call coder::search with payload:
      """
      {"query":".env","path":".","search_content":true,"search_paths":true}
      """
    Then the search has no path match for ".env"

  Scenario: invalid regex fails clearly
    Given a jailed code surface
    When I call coder::search with payload:
      """
      {"query":"[","path":".","regex":true}
      """
    Then the call failed with code "C210"

  Scenario: empty queries fail clearly
    Given a jailed code surface
    When I call coder::search with payload:
      """
      {"query":"","path":"."}
      """
    Then the call failed with code "C210"
