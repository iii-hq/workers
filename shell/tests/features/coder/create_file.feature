@direct @create-file
Feature: coder create-file adversarial behavior

  Scenario: mixed batch preserves independent success and protected failure
    Given a jailed code surface
    When I call coder::create-file with payload:
      """
      {"files":[{"path":"ok/a.txt","content":"a","parents":true},{"path":"secrets/key.txt","content":"secret","parents":true}]}
      """
    Then the result for "ok/a.txt" succeeded
    And the result for "secrets/key.txt" failed with code "C211"
    And the file "ok/a.txt" exists
    And the file "secrets/key.txt" does not exist

  Scenario: existing files require overwrite to replace
    Given a jailed code surface
    And a file at "same.txt" with content:
      """
      old
      """
    When I call coder::create-file with payload:
      """
      {"files":[{"path":"same.txt","content":"new","overwrite":false}]}
      """
    Then the result for "same.txt" failed with code "C213"
    And the file "same.txt" equals:
      """
      old
      """
    When I call coder::create-file with payload:
      """
      {"files":[{"path":"same.txt","content":"new\n","overwrite":true}]}
      """
    Then the result for "same.txt" succeeded
    And the file "same.txt" equals:
      """
      new
      """

  Scenario: empty create batches fail the whole call
    Given a jailed code surface
    When I call coder::create-file with payload:
      """
      {"files":[]}
      """
    Then the call failed with code "C210"
