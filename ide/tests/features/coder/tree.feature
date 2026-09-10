@direct @tree
Feature: coder tree adversarial behavior

  Scenario: max depth returns a truncation hint instead of descending forever
    Given a jailed code surface
    And a file at "deep/child/file.txt" with content:
      """
      x
      """
    When I call coder::tree with payload:
      """
      {"path":".","max_depth":1,"per_folder_limit":3}
      """
    Then the tree contains "deep"
    And the tree marks "deep" truncated for "max_depth"
    And the tree does not contain "file.txt"

  Scenario: per-folder limits are explicit
    Given a jailed code surface
    And a file at "wide/a.txt" with content:
      """
      a
      """
    And a file at "wide/b.txt" with content:
      """
      b
      """
    And a file at "wide/c.txt" with content:
      """
      c
      """
    When I call coder::tree with payload:
      """
      {"path":"wide","max_depth":1,"per_folder_limit":2}
      """
    Then the tree marks "wide" truncated for "per_folder_limit"

  Scenario: default excludes surface as truncated stubs
    Given a jailed code surface
    And a file at "node_modules/pkg/index.js" with content:
      """
      module.exports = 1
      """
    When I call coder::tree with payload:
      """
      {"path":".","max_depth":2}
      """
    Then the tree contains "node_modules"
    And the tree marks "node_modules" truncated for "default_exclude"
    And the tree does not contain "index.js"
