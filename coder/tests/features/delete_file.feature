@engine @delete
Feature: coder::delete-file
  Per-path results in `results[]`. Missing paths are idempotent
  successes with `removed: false`. Directories need `recursive: true`;
  recursive deletes refuse to descend through non-accessible entries
  and the worker refuses to delete an allowed root itself.

  Background:
    Given the iii engine is reachable

  Scenario: delete an existing file
    Given a file at "scratch.txt" with content:
      """
      bye
      """
    When I call coder::delete-file with payload:
      """
      {"paths": ["scratch.txt"]}
      """
    Then the result for "scratch.txt" succeeded
    And the result for "scratch.txt" was removed
    And the file "scratch.txt" does not exist on disk

  Scenario: deleting a missing path is an idempotent success
    When I call coder::delete-file with payload:
      """
      {"paths": ["never_existed.txt"]}
      """
    Then the result for "never_existed.txt" succeeded
    And the result for "never_existed.txt" was not removed

  Scenario: deleting a non-empty directory without recursive fails with C216
    Given a file at "subdir/inner.txt" with content:
      """
      x
      """
    When I call coder::delete-file with payload:
      """
      {"paths": ["subdir"], "recursive": false}
      """
    Then the result for "subdir" failed with code "C216"

  Scenario: recursive directory delete removes the whole tree
    Given a file at "tree/a.txt" with content:
      """
      a
      """
    And a file at "tree/inner/b.txt" with content:
      """
      b
      """
    When I call coder::delete-file with payload:
      """
      {"paths": ["tree"], "recursive": true}
      """
    Then the result for "tree" succeeded
    And the result for "tree" was removed
    And the file "tree/a.txt" does not exist on disk
    And the file "tree/inner/b.txt" does not exist on disk

  Scenario: recursive delete blocked by a non-accessible child fails with C211
    Given a file at "mixed/ok.txt" with content:
      """
      ok
      """
    And a file at "mixed/.env" with content:
      """
      secret
      """
    When I call coder::delete-file with payload:
      """
      {"paths": ["mixed"], "recursive": true}
      """
    Then the result for "mixed" failed with code "C211"

  Scenario: deleting a non-accessible file directly fails with C211
    Given a file at ".env" with content:
      """
      secret
      """
    When I call coder::delete-file with payload:
      """
      {"paths": [".env"]}
      """
    Then the result for ".env" failed with code "C211"

  Scenario: deleting an allowed root itself fails with C210
    When I call coder::delete-file with payload:
      """
      {"paths": ["."]}
      """
    Then the result for "." failed with code "C210"

  Scenario: an empty paths array is a top-level C210
    When I call coder::delete-file with payload:
      """
      {"paths": []}
      """
    Then the call failed with code "C210"
