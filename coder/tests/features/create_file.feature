@engine @create
Feature: coder::create-file
  Per-file results are reported in a `results[]` array so a single bad
  input never aborts the batch. Non-accessible paths return `C211`,
  existing files without `overwrite` return `C217`, and an empty
  `files` array is a top-level `C210`.

  Background:
    Given the iii engine is reachable

  Scenario: create a single file with parent directories
    When I call coder::create-file with payload:
      """
      {
        "files": [
          {"path": "notes/intro.md", "content": "# hello\n", "mode": "0644", "parents": true, "overwrite": false}
        ]
      }
      """
    Then the call succeeded
    And the result for "notes/intro.md" succeeded
    And the result for "notes/intro.md" wrote 8 bytes
    And the file "notes/intro.md" exists on disk
    And the file "notes/intro.md" on disk contains "hello"

  Scenario: batch create two files reports per-file success
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "a.txt", "content": "a", "mode": "0644", "parents": true, "overwrite": false},
        {"path": "b.txt", "content": "bb", "mode": "0644", "parents": true, "overwrite": false}
      ]}
      """
    Then the result for "a.txt" succeeded
    And the result for "a.txt" wrote 1 bytes
    And the result for "b.txt" succeeded
    And the result for "b.txt" wrote 2 bytes

  Scenario: create without overwrite on an existing file fails with C217
    Given a file at "exists.txt" with content:
      """
      old
      """
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "exists.txt", "content": "new", "mode": "0644", "parents": true, "overwrite": false},
        {"path": "fresh.txt", "content": "fresh", "mode": "0644", "parents": true, "overwrite": false}
      ]}
      """
    Then the call succeeded
    And the result for "exists.txt" failed with code "C217"
    And the result for "fresh.txt" succeeded
    And the file "exists.txt" on disk contains "old"

  Scenario: overwrite replaces existing content
    Given a file at "exists.txt" with content:
      """
      original
      """
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "exists.txt", "content": "replaced", "mode": "0644", "parents": true, "overwrite": true}
      ]}
      """
    Then the result for "exists.txt" succeeded
    And the file "exists.txt" on disk contains "replaced"

  Scenario: creating a non-accessible file fails with C211
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": ".env", "content": "API_KEY=secret", "mode": "0644", "parents": true, "overwrite": true}
      ]}
      """
    Then the call succeeded
    And the result for ".env" failed with code "C211"
    And the file ".env" does not exist on disk

  Scenario: creating a non-accessible file under a subdirectory also fails with C211
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "secrets/key.pem", "content": "x", "mode": "0644", "parents": true, "overwrite": false}
      ]}
      """
    Then the result for "secrets/key.pem" failed with code "C211"

  Scenario: an empty files array is a top-level C210
    When I call coder::create-file with payload:
      """
      {"files": []}
      """
    Then the call failed with code "C210"
