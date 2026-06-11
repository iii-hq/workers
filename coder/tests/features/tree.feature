@engine @tree
Feature: coder::tree
  Recursive directory snapshot bounded by `max_depth` and a
  `per_folder_limit`. Folders that hit the limit are tagged with a
  `truncated` block; deep subtrees cut off by `max_depth` carry the
  same metadata so callers can switch to `coder::list-folder` for
  pagination.

  Background:
    Given the iii engine is reachable

  Scenario: nested directory snapshot returns a child hierarchy
    Given a file at "top.txt" with content:
      """
      t
      """
    And a file at "sub/inner.txt" with content:
      """
      i
      """
    When I call coder::tree with payload:
      """
      {"path": "."}
      """
    Then the call succeeded
    And the tree has a node at "top.txt"
    And the tree node at "top.txt" has kind "file"
    And the tree has a node at "sub"
    And the tree node at "sub" has kind "dir"
    And the tree has a node at "sub/inner.txt"
    And the tree node at "sub/inner.txt" has kind "file"

  Scenario: max_depth truncates the subtree with a list-folder hint
    Given a file at "a/b/c/d.txt" with content:
      """
      deep
      """
    When I call coder::tree with payload:
      """
      {"path": ".", "max_depth": 2}
      """
    Then the tree has a node at "a/b"
    And the tree node at "a/b" is truncated with reason "max_depth"
    And the tree node at "a/b" hint mentions "max_depth"
    And the tree has no node at "a/b/c"

  Scenario: per_folder_limit truncates wide folders
    Given a file at "one.txt" with content:
      """
      1
      """
    And a file at "two.txt" with content:
      """
      2
      """
    And a file at "three.txt" with content:
      """
      3
      """
    When I call coder::tree with payload:
      """
      {"path": ".", "per_folder_limit": 2}
      """
    Then the tree node at "." has 2 children
    And the tree node at "." is truncated with reason "per_folder_limit"
    And the tree node at "." hint mentions "list-folder"

  Scenario: non-accessible entries appear with the flag
    Given a file at ".env" with content:
      """
      secret
      """
    When I call coder::tree with payload:
      """
      {"path": "."}
      """
    Then the tree has a node at ".env"
    And the tree node at ".env" is non_accessible

  Scenario: rooting the tree at a subpath returns only that subtree
    Given a file at "outside.txt" with content:
      """
      o
      """
    And a file at "sub/inside.txt" with content:
      """
      i
      """
    When I call coder::tree with payload:
      """
      {"path": "sub"}
      """
    Then the tree has a node at "inside.txt"
    And the tree has no node at "outside.txt"

  Scenario: default-excluded directories appear as childless stubs
    Given a file at "node_modules/pkg/index.js" with content:
      """
      x
      """
    And a file at "src/main.rs" with content:
      """
      fn main() {}
      """
    When I call coder::tree with payload:
      """
      {"path": "."}
      """
    Then the call succeeded
    And the tree has a node at "node_modules"
    And the tree node at "node_modules" is truncated with reason "default_exclude"
    And the tree node at "node_modules" hint mentions "use_default_excludes"
    And the tree has no node at "node_modules/pkg"
    And the tree has a node at "src/main.rs"

  Scenario: use_default_excludes false descends into excluded directories
    Given a file at "node_modules/pkg/index.js" with content:
      """
      x
      """
    When I call coder::tree with payload:
      """
      {"path": ".", "use_default_excludes": false}
      """
    Then the call succeeded
    And the tree has a node at "node_modules/pkg/index.js"

  Scenario: tree on a missing folder fails with C211
    When I call coder::tree with payload:
      """
      {"path": "missing"}
      """
    Then the call failed with code "C211"
