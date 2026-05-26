@engine @list
Feature: coder::list-folder
  Paginated single-folder listing sorted by name. Non-accessible
  entries are still reported with `non_accessible: true` so callers
  can see they exist even though they cannot be read or written.
  Test config sets `list_default_page_size` to 5 and
  `list_max_page_size` to 100.

  Background:
    Given the iii engine is reachable

  Scenario: list a flat folder sorted by name
    Given a file at "alpha.txt" with content:
      """
      a
      """
    And a file at "bravo.txt" with content:
      """
      b
      """
    And a directory at "charlie"
    When I call coder::list-folder with payload:
      """
      {"path": "."}
      """
    Then the call succeeded
    And the listing has an entry named "alpha.txt"
    And the listing entry "alpha.txt" has kind "file"
    And the listing has an entry named "bravo.txt"
    And the listing entry "bravo.txt" has kind "file"
    And the listing has an entry named "charlie"
    And the listing entry "charlie" has kind "dir"
    And the listing total equals 3
    And the listing has_more is false

  Scenario: pagination splits entries across pages with has_more
    Given a file at "f1.txt" with content:
      """
      1
      """
    And a file at "f2.txt" with content:
      """
      2
      """
    And a file at "f3.txt" with content:
      """
      3
      """
    And a file at "f4.txt" with content:
      """
      4
      """
    And a file at "f5.txt" with content:
      """
      5
      """
    And a file at "f6.txt" with content:
      """
      6
      """
    And a file at "f7.txt" with content:
      """
      7
      """
    When I call coder::list-folder with payload:
      """
      {"path": ".", "page": 1}
      """
    Then the listing total equals 7
    And the listing has 5 entries
    And the listing page equals 1
    And the listing page_size equals 5
    And the listing has_more is true
    When I call coder::list-folder with payload:
      """
      {"path": ".", "page": 2}
      """
    Then the listing total equals 7
    And the listing has 2 entries
    And the listing page equals 2
    And the listing has_more is false

  Scenario: explicit page_size overrides the default
    Given a file at "a.txt" with content:
      """
      x
      """
    And a file at "b.txt" with content:
      """
      x
      """
    And a file at "c.txt" with content:
      """
      x
      """
    When I call coder::list-folder with payload:
      """
      {"path": ".", "page": 1, "page_size": 2}
      """
    Then the listing has 2 entries
    And the listing page_size equals 2
    And the listing has_more is true

  Scenario: page_size is capped at list_max_page_size
    Given a file at "only.txt" with content:
      """
      x
      """
    When I call coder::list-folder with payload:
      """
      {"path": ".", "page": 1, "page_size": 100000}
      """
    Then the listing page_size equals 100

  Scenario: non-accessible entries are listed with the flag
    Given a file at ".env" with content:
      """
      secret
      """
    And a file at "public.txt" with content:
      """
      ok
      """
    When I call coder::list-folder with payload:
      """
      {"path": "."}
      """
    Then the listing has an entry named ".env"
    And the listing entry ".env" is non_accessible
    And the listing entry "public.txt" is accessible

  Scenario: listing a missing folder fails with C211
    When I call coder::list-folder with payload:
      """
      {"path": "no_such_dir"}
      """
    Then the call failed with code "C211"
