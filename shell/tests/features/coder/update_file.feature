@direct @update-file
Feature: coder update-file adversarial behavior

  Scenario: line operations apply against original line numbers
    Given a jailed code surface
    And a file at "edit.txt" with content:
      """
      alpha
      beta
      gamma
      delta
      """
    When I call coder::update-file with payload:
      """
      {"files":[{"path":"edit.txt","ops":[{"op":"insert","at_line":2,"content":"inserted\n"},{"op":"update_lines","from_line":3,"to_line":3,"content":"GAMMA\n"},{"op":"remove","from_line":4,"to_line":4}]}]}
      """
    Then the result for "edit.txt" succeeded
    And the file "edit.txt" equals:
      """
      alpha
      inserted
      beta
      GAMMA
      """

  Scenario: regex expect_matches mismatch fails atomically
    Given a jailed code surface
    And a file at "replace.txt" with content:
      """
      foo foo
      """
    When I call coder::update-file with payload:
      """
      {"files":[{"path":"replace.txt","ops":[{"op":"replace","pattern":"foo","replacement":"bar","expect_matches":1}]}]}
      """
    Then the result for "replace.txt" failed with code "C210"
    And the file "replace.txt" equals:
      """
      foo foo
      """

  Scenario: overlapping line edits are rejected before write
    Given a jailed code surface
    And a file at "overlap.txt" with content:
      """
      one
      two
      three
      """
    When I call coder::update-file with payload:
      """
      {"files":[{"path":"overlap.txt","ops":[{"op":"remove","from_line":1,"to_line":2},{"op":"update_lines","from_line":2,"to_line":3,"content":"x\n"}]}]}
      """
    Then the result for "overlap.txt" failed with code "C210"

  Scenario: protected files cannot be edited
    Given a jailed code surface
    And a file at ".env" with content:
      """
      TOKEN=secret
      """
    When I call coder::update-file with payload:
      """
      {"files":[{"path":".env","ops":[{"op":"replace","pattern":"secret","replacement":"redacted"}]}]}
      """
    Then the result for ".env" failed with code "C211"
