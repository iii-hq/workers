@direct @read-file
Feature: coder read-file adversarial behavior

  Scenario: single-file windows preserve line addressing and content bounds
    Given a jailed code surface
    And a file at "story.txt" with content:
      """
      one
      two
      three
      four
      """
    When I call coder::read-file with payload:
      """
      {"path":"story.txt","line_from":2,"line_to":3,"numbered":true}
      """
    Then the read content contains "two"
    And the read content contains "three"
    And the read field "more_lines" is true
    And the read field "lines_returned" equals 2

  Scenario: stat probes return metadata without consuming content
    Given a jailed code surface
    And a file at "meta.txt" with content:
      """
      abc
      """
    When I call coder::read-file with payload:
      """
      {"path":"meta.txt","stat":true}
      """
    Then the call succeeded
    And the read field "size" equals 4
    And the read field "lines_returned" equals 0

  Scenario: invalid UTF-8 is surfaced as lossy text
    Given a jailed code surface
    And a binary file at "bin.dat" with invalid UTF-8 bytes
    When I call coder::read-file with payload:
      """
      {"path":"bin.dat"}
      """
    Then the call succeeded
    And the read field "is_utf8" is false

  Scenario: batch reads classify budget exhaustion per entry
    Given a jailed code surface
    And a file at "batch/a.txt" with 32 lines of 5 bytes each
    And a file at "batch/b.txt" with content:
      """
      b
      """
    When I call coder::read-file with payload:
      """
      {"paths":["batch/a.txt","batch/b.txt"]}
      """
    Then the batch read result for "batch/a.txt" succeeded
    And the batch read result for "batch/b.txt" failed with code "C218"

  Scenario: path and paths are mutually exclusive
    Given a jailed code surface
    When I call coder::read-file with payload:
      """
      {"path":"a.txt","paths":["b.txt"]}
      """
    Then the call failed with code "C210"
