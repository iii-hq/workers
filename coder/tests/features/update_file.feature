@engine @update
Feature: coder::update-file
  Batched line-oriented and regex edits (insert / remove / update_lines /
  replace) applied bottom-up for line ops so earlier ops still reference
  the original line numbers. Regex replace runs after line ops. Each file
  commits atomically; overlapping line ops in one file reject with `C210`
  and leave the file unchanged.

  Background:
    Given the iii engine is reachable

  Scenario: insert a line at a given position
    Given a file at "doc.txt" with content:
      """
      one
      two
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "insert", "at_line": 2, "content": "inserted"}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    And the result for "doc.txt" applied 1 ops
    And the result for "doc.txt" has line count 3

  Scenario: remove a single line
    Given a file at "doc.txt" with content:
      """
      one
      two
      three
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "remove", "from_line": 2, "to_line": 2}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    And the result for "doc.txt" has line count 2

  Scenario: update_lines a single line and read it back
    Given a file at "doc.txt" with content:
      """
      one
      two
      three
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "update_lines", "from_line": 2, "to_line": 2, "content": "REPLACED"}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    When I call coder::read-file with payload:
      """
      {"path": "doc.txt"}
      """
    Then the read content equals:
      """
      one
      REPLACED
      three
      """

  Scenario: multiple ops applied bottom-up keep earlier line numbers stable
    Given a file at "doc.txt" with content:
      """
      A
      B
      C
      D
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "remove", "from_line": 4, "to_line": 4},
          {"op": "update_lines", "from_line": 2, "to_line": 2, "content": "BB"}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    And the result for "doc.txt" applied 2 ops
    When I call coder::read-file with payload:
      """
      {"path": "doc.txt"}
      """
    Then the read content equals:
      """
      A
      BB
      C
      """

  Scenario: overlapping ops are rejected per file with C210
    Given a file at "doc.txt" with content:
      """
      A
      B
      C
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "remove", "from_line": 1, "to_line": 2},
          {"op": "update_lines", "from_line": 2, "to_line": 3, "content": "X"}
        ]}
      ]}
      """
    Then the result for "doc.txt" failed with code "C210"

  Scenario: batch partial success - one file ok, one missing
    Given a file at "ok.txt" with content:
      """
      hello
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "ok.txt", "ops": [
          {"op": "update_lines", "from_line": 1, "to_line": 1, "content": "HI"}
        ]},
        {"path": "missing.txt", "ops": [
          {"op": "insert", "at_line": 1, "content": "x"}
        ]}
      ]}
      """
    Then the result for "ok.txt" succeeded
    And the result for "missing.txt" failed with code "C211"

  Scenario: updating a non-accessible file fails with C211
    Given a file at ".env" with content:
      """
      KEY=v
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": ".env", "ops": [
          {"op": "update_lines", "from_line": 1, "to_line": 1, "content": "OTHER=z"}
        ]}
      ]}
      """
    Then the result for ".env" failed with code "C211"

  Scenario: an empty files array is a top-level C210
    When I call coder::update-file with payload:
      """
      {"files": []}
      """
    Then the call failed with code "C210"

  Scenario: regex replace all occurrences
    Given a file at "doc.txt" with content:
      """
      foo bar foo
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "replace", "pattern": "foo", "replacement": "baz"}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    When I call coder::read-file with payload:
      """
      {"path": "doc.txt"}
      """
    Then the read content equals:
      """
      baz bar baz
      """

  Scenario: regex replace with capture groups
    Given a file at "doc.txt" with content:
      """
      HOST=8080
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "replace", "pattern": "(\\w+)=(\\d+)", "replacement": "$1: $2"}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    When I call coder::read-file with payload:
      """
      {"path": "doc.txt"}
      """
    Then the read content equals:
      """
      HOST: 8080
      """

  Scenario: invalid regex is rejected with C210
    Given a file at "doc.txt" with content:
      """
      hello
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "replace", "pattern": "[unclosed", "replacement": "x"}
        ]}
      ]}
      """
    Then the result for "doc.txt" failed with code "C210"

  Scenario: mixed update_lines then regex replace
    Given a file at "doc.txt" with content:
      """
      OLD
      keep
      OLD
      """
    When I call coder::update-file with payload:
      """
      {"files": [
        {"path": "doc.txt", "ops": [
          {"op": "remove", "from_line": 2, "to_line": 2},
          {"op": "replace", "pattern": "OLD", "replacement": "NEW"}
        ]}
      ]}
      """
    Then the result for "doc.txt" succeeded
    And the result for "doc.txt" applied 2 ops
    When I call coder::read-file with payload:
      """
      {"path": "doc.txt"}
      """
    Then the read content equals:
      """
      NEW
      NEW
      """
