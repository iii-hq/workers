@engine @security
Feature: path-jail invariants
  Relative wire paths are interpreted against the primary root; absolute
  wire paths are accepted only when they canonicalize inside an allowed
  root. Absolute paths outside all roots, `..` escapes, and crafted
  symlinks cannot leave the jail (`C215`); non-accessible globs block
  reads even though the entry is still listed (`C211`).

  Background:
    Given the iii engine is reachable

  Scenario: a dot-dot escape on read fails with C215
    When I call coder::read-file with payload:
      """
      {"path": "../../etc/passwd"}
      """
    Then the call failed with code "C215"

  Scenario: a dot-dot escape on create is reported per item as C215
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "../escape.txt", "content": "x", "mode": "0644", "parents": false, "overwrite": false}
      ]}
      """
    Then the call succeeded
    And the result for "../escape.txt" failed with code "C215"

  Scenario: an absolute path outside all roots fails with C215 on read
    When I call coder::read-file with payload:
      """
      {"path": "/etc/passwd"}
      """
    Then the call failed with code "C215"

  Scenario: an absolute path outside all roots is rejected per item on create
    When I call coder::create-file with payload:
      """
      {"files": [
        {"path": "/tmp/abs.txt", "content": "x", "mode": "0644", "parents": false, "overwrite": false}
      ]}
      """
    Then the result for "/tmp/abs.txt" failed with code "C215"

  @unix
  Scenario: a symlink whose target escapes the allowed roots is rejected with C215
    Given a symlink at "escape_link" pointing to a path outside base
    When I call coder::read-file with payload:
      """
      {"path": "escape_link"}
      """
    Then the call failed with code "C215"
