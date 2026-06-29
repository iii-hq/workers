@direct @delete-file
Feature: coder delete-file adversarial behavior

  Scenario: files and empty directories delete independently
    Given a jailed code surface
    And a file at "remove/file.txt" with content:
      """
      doomed
      """
    And a directory at "remove/empty"
    When I call coder::delete-file with payload:
      """
      {"paths":["remove/file.txt","remove/empty"]}
      """
    Then the result for "remove/file.txt" succeeded
    And the result for "remove/empty" succeeded
    And the file "remove/file.txt" does not exist
    And the file "remove/empty" does not exist

  Scenario: missing files are idempotent successes
    Given a jailed code surface
    When I call coder::delete-file with payload:
      """
      {"paths":["already-gone.txt"]}
      """
    Then the result for "already-gone.txt" succeeded
    And the file "already-gone.txt" does not exist

  Scenario: recursive delete refuses subtrees containing protected entries
    Given a jailed code surface
    And a file at "bundle/keep.txt" with content:
      """
      keep
      """
    And a file at "bundle/.env" with content:
      """
      TOKEN=secret
      """
    When I call coder::delete-file with payload:
      """
      {"paths":["bundle"],"recursive":true}
      """
    Then the result for "bundle" failed with code "C211"
    And the file "bundle/keep.txt" exists
