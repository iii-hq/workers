@direct @move
Feature: coder move adversarial behavior

  Scenario: same-root file moves preserve content
    Given a jailed code surface
    And a file at "old/name.txt" with content:
      """
      moved
      """
    When I call coder::move with payload:
      """
      {"files":[{"from":"old/name.txt","to":"new/name.txt","parents":true}]}
      """
    Then the move from "old/name.txt" to "new/name.txt" succeeded
    And the file "old/name.txt" does not exist
    And the file "new/name.txt" contains "moved"

  Scenario: destination collisions require overwrite
    Given a jailed code surface
    And a file at "from.txt" with content:
      """
      from
      """
    And a file at "to.txt" with content:
      """
      to
      """
    When I call coder::move with payload:
      """
      {"files":[{"from":"from.txt","to":"to.txt","overwrite":false}]}
      """
    Then the move from "from.txt" to "to.txt" failed with code "C213"
    And the file "from.txt" exists
    And the file "to.txt" equals:
      """
      to
      """

  Scenario: cross-root file moves are allowed
    Given a jailed code surface
    And a file at "cross.txt" with content:
      """
      cross-root
      """
    When I call coder::move with payload:
      """
      {"files":[{"from":"cross.txt","to":"{{secondary}}/cross.txt","overwrite":true}]}
      """
    Then the move from "cross.txt" to "{{secondary}}/cross.txt" succeeded
    And the file "cross.txt" does not exist
    And the file "{{secondary}}/cross.txt" contains "cross-root"

  Scenario: cross-root directories are rejected
    Given a jailed code surface
    And a directory at "dir"
    When I call coder::move with payload:
      """
      {"files":[{"from":"dir","to":"{{secondary}}/dir"}]}
      """
    Then the move from "dir" to "{{secondary}}/dir" failed with code "C210"
