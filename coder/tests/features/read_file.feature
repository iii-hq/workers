@engine @read
Feature: coder::read-file
  Returns the file contents plus `size`, `mode`, `mtime`, and an
  `is_utf8` flag. The path is echoed back for caller correlation.
  Capped by `max_read_bytes`; non-accessible paths return `C211`.

  Background:
    Given the iii engine is reachable

  Scenario: read a UTF-8 text file
    Given a file at "hello.txt" with content:
      """
      hello world
      """
    When I call coder::read-file with payload:
      """
      {"path": "hello.txt"}
      """
    Then the call succeeded
    And the read path equals "hello.txt"
    And the read content equals:
      """
      hello world
      """
    And the read size equals 12
    And the read is_utf8 is true

  Scenario: reading a missing file fails with C211
    When I call coder::read-file with payload:
      """
      {"path": "nope.txt"}
      """
    Then the call failed with code "C211"

  Scenario: reading a directory fails with C210
    Given a directory at "subdir"
    When I call coder::read-file with payload:
      """
      {"path": "subdir"}
      """
    Then the call failed with code "C210"

  Scenario: reading a non-accessible file fails with C211 even though list-folder shows it
    Given a file at ".env" with content:
      """
      API_KEY=secret
      """
    When I call coder::list-folder with payload:
      """
      {"path": "."}
      """
    Then the listing has an entry named ".env"
    And the listing entry ".env" is non_accessible
    When I call coder::read-file with payload:
      """
      {"path": ".env"}
      """
    Then the call failed with code "C211"

  Scenario: reading an oversize file fails with C213
    Given a file at "big.bin" with 5000 bytes of content
    When I call coder::read-file with payload:
      """
      {"path": "big.bin"}
      """
    Then the call failed with code "C213"
