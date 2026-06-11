@engine @search
Feature: coder::search
  Combined path + content search under the allowed roots. Supports literal
  and regex queries with include / exclude globs, returns content and
  path matches in separate arrays, and refuses to read non-accessible
  files. Binary files (NUL byte heuristic) and oversize files are
  skipped silently. Noise paths matching default_exclude_globs are
  skipped by default (use_default_excludes: false searches inside);
  optional context lines (max 10 each way) ride along with content
  matches, and the response is bounded by a byte budget that flags
  truncated instead of erroring.

  Background:
    Given the iii engine is reachable

  Scenario: literal content match reports path, line, and column
    Given a file at "notes.md" with content:
      """
      hello world
      goodbye world
      """
    When I call coder::search with payload:
      """
      {"query": "goodbye"}
      """
    Then the call succeeded
    And the search has a content match for "notes.md" at line 2
    And the search truncated is false

  Scenario: regex content match
    Given a file at "phones.txt" with content:
      """
      call 555-1234 or 555-9876
      """
    When I call coder::search with payload:
      """
      {"query": "\\d{3}-\\d{4}", "regex": true}
      """
    Then the search has a content match for "phones.txt"

  Scenario: ignore_case literal matches mixed-case occurrences
    Given a file at "doc.md" with content:
      """
      Hello world
      """
    When I call coder::search with payload:
      """
      {"query": "hello", "ignore_case": true}
      """
    Then the search has a content match for "doc.md"

  Scenario: search by path only (search_content false)
    Given a file at "notes.md" with content:
      """
      irrelevant body
      """
    When I call coder::search with payload:
      """
      {"query": "notes", "search_content": false, "search_paths": true}
      """
    Then the search has a path match for "notes.md"
    And the search has no content matches

  Scenario: include_globs limits which files are scanned
    Given a file at "src/main.rs" with content:
      """
      fn main() {}
      """
    And a file at "src/lib.md" with content:
      """
      fn main() {}
      """
    When I call coder::search with payload:
      """
      {"query": "main", "include_globs": ["**/*.rs"], "search_paths": false}
      """
    Then the search has a content match for "src/main.rs"
    And the search has no content match for "src/lib.md"

  Scenario: exclude_globs removes matching files from results
    Given a file at "keep.txt" with content:
      """
      banana
      """
    And a file at "skip.txt" with content:
      """
      banana
      """
    When I call coder::search with payload:
      """
      {"query": "banana", "exclude_globs": ["skip.txt"]}
      """
    Then the search has a content match for "keep.txt"
    And the search has no content match for "skip.txt"

  Scenario: path scopes the search to a subdirectory
    Given a file at "a/hit.txt" with content:
      """
      banana
      """
    And a file at "b/hit.txt" with content:
      """
      banana
      """
    When I call coder::search with payload:
      """
      {"query": "banana", "path": "a"}
      """
    Then the search has a content match for "a/hit.txt"
    And the search has no content match for "b/hit.txt"

  Scenario: empty query is rejected with C210
    When I call coder::search with payload:
      """
      {"query": ""}
      """
    Then the call failed with code "C210"

  Scenario: non-accessible files are excluded from content and path results
    Given a file at ".env" with content:
      """
      banana
      """
    And a file at "ok.txt" with content:
      """
      banana
      """
    When I call coder::search with payload:
      """
      {"query": "banana"}
      """
    Then the search has a content match for "ok.txt"
    And the search has no content match for ".env"
    And the search has no path match for ".env"

  Scenario: max_matches truncates the result set
    Given a file at "a.txt" with content:
      """
      hit
      """
    And a file at "b.txt" with content:
      """
      hit
      """
    And a file at "c.txt" with content:
      """
      hit
      """
    When I call coder::search with payload:
      """
      {"query": "hit", "max_matches": 2}
      """
    Then the search truncated is true

  Scenario: binary files are skipped silently
    Given a binary file at "blob.bin" containing "needle"
    And a file at "readme.txt" with content:
      """
      needle here
      """
    When I call coder::search with payload:
      """
      {"query": "needle", "search_paths": false}
      """
    Then the search has a content match for "readme.txt"
    And the search has no content match for "blob.bin"

  Scenario: search_content and search_paths both false is rejected with C210
    When I call coder::search with payload:
      """
      {"query": "x", "search_content": false, "search_paths": false}
      """
    Then the call failed with code "C210"

  Scenario: context_lines_before above the cap is rejected with C210
    When I call coder::search with payload:
      """
      {"query": "x", "context_lines_before": 11}
      """
    Then the call failed with code "C210"

  Scenario: context lines within the cap still succeed
    Given a file at "ctx.txt" with content:
      """
      one
      two needle
      three
      """
    When I call coder::search with payload:
      """
      {"query": "needle", "context_lines_before": 1, "context_lines_after": 1}
      """
    Then the call succeeded
    And the search has a content match for "ctx.txt" at line 2

  Scenario: default excludes hide node_modules from content and path results
    Given a file at "node_modules/pkg/dep.js" with content:
      """
      banana
      """
    And a file at "src/ok.txt" with content:
      """
      banana
      """
    When I call coder::search with payload:
      """
      {"query": "banana"}
      """
    Then the search has a content match for "src/ok.txt"
    And the search has no content match for "node_modules/pkg/dep.js"
    And the search has no path match for "node_modules/pkg/dep.js"

  Scenario: use_default_excludes false searches inside excluded folders
    Given a file at "node_modules/pkg/dep.js" with content:
      """
      banana
      """
    When I call coder::search with payload:
      """
      {"query": "banana", "use_default_excludes": false}
      """
    Then the search has a content match for "node_modules/pkg/dep.js"
