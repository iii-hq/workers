@engine @lifecycle
Feature: end-to-end coder lifecycle
  Decomposes the existing `tests/integration.rs` subprocess scenario
  into readable Gherkin: create -> read -> update -> read -> list ->
  tree -> search -> non-accessible blocked -> delete. Uses the
  in-process registration so the engine drives the same handlers a
  production binary would.

  Background:
    Given the iii engine is reachable

  Scenario: full create-update-search-delete journey for one file
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "hello.txt", "content": "hello world\nsecond line\n", "mode": "0644", "parents": true, "overwrite": false}
      ]}
      """
    Then the result for "hello.txt" succeeded
    And the result for "hello.txt" wrote 24 bytes

    When I call coder::read-file with payload:
      """
      {"path": "hello.txt"}
      """
    Then the read size equals 24
    And the read content equals:
      """
      hello world
      second line
      """

    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "hello.txt", "ops": [
          {"op": "update_lines", "from_line": 2, "to_line": 2, "content": "REPLACED"}
        ]}
      ]}
      """
    Then the result for "hello.txt" succeeded

    When I call coder::read-file with payload:
      """
      {"path": "hello.txt"}
      """
    Then the read content equals:
      """
      hello world
      REPLACED
      """

    When I call coder::list-folder with payload:
      """
      {"path": "."}
      """
    Then the listing has an entry named "hello.txt"
    And the listing entry "hello.txt" has kind "file"

    When I call coder::tree with payload:
      """
      {"path": "."}
      """
    Then the tree has a node at "hello.txt"
    And the tree node at "hello.txt" has kind "file"

    When I call coder::search with payload:
      """
      {"query": "REPLACED"}
      """
    Then the search has a content match for "hello.txt" at line 2

    Given a file at ".env" with content:
      """
      API_KEY=secret
      """
    When I call coder::list-folder with payload:
      """
      {"path": "."}
      """
    Then the listing has an entry named ".env"
    And the listing entry ".env" is non_accessible
    When I call coder::read-file with payload:
      """
      {"path": ".env"}
      """
    Then the call failed with code "C211"

    When I call coder::delete-file with payload:
      """
      {"paths": ["hello.txt"]}
      """
    Then the result for "hello.txt" succeeded
    And the result for "hello.txt" was removed
    And the file "hello.txt" does not exist on disk
