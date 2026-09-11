@direct @path-security
Feature: coder path security

  Scenario: absolute paths outside every allowed root are rejected
    Given a jailed code surface
    When I call coder::create-file with payload:
      """
      {"files":[{"path":"{{outside}}/escape.txt","content":"escape","overwrite":true}]}
      """
    Then the call failed with code "C215"

  Scenario: protected globs are listable but not readable or writable
    Given a jailed code surface
    And a file at ".env" with content:
      """
      TOKEN=secret
      """
    When I call coder::list-folder with payload:
      """
      {"path":"."}
      """
    Then the listing contains ".env"
    And the listing marks ".env" non-accessible
    When I call coder::read-file with payload:
      """
      {"path":".env"}
      """
    Then the call failed with code "C211"
    When I call coder::create-file with payload:
      """
      {"files":[{"path":".env","content":"replace","overwrite":true}]}
      """
    Then the result for ".env" failed with code "C211"

  Scenario: symlink escapes are rejected after canonicalization
    Given a jailed code surface
    And a symlink at "links/outside.txt" pointing outside the jail
    When I call coder::read-file with payload:
      """
      {"path":"links/outside.txt"}
      """
    Then the call failed with code "C215"

  Scenario: session filesystem scope cannot reach sibling files
    Given a jailed code surface
    And a session directory at "session"
    And a file at "outside-session.txt" with content:
      """
      sibling
      """
    When I call coder::read-file with payload:
      """
      {"path":"../outside-session.txt","fs_scope":{"root":"{{session}}","boundary":"workspace"}}
      """
    Then the call failed with code "C220"
