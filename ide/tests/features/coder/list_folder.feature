@direct @list-folder
Feature: coder list-folder adversarial behavior

  Scenario: pagination is stable and protected entries stay visible
    Given a jailed code surface
    And a file at "items/a.txt" with content:
      """
      a
      """
    And a file at "items/b.txt" with content:
      """
      b
      """
    And a file at "items/c.txt" with content:
      """
      c
      """
    And a file at "items/.env" with content:
      """
      TOKEN=secret
      """
    When I call coder::list-folder with payload:
      """
      {"path":"items","page":1,"page_size":2}
      """
    Then the listing contains ".env"
    And the listing contains "a.txt"
    And the listing marks ".env" non-accessible
    And the listing has more pages

  Scenario: page size is clamped by config
    Given a jailed code surface
    And a file at "many/a.txt" with content:
      """
      a
      """
    When I call coder::list-folder with payload:
      """
      {"path":"many","page_size":99}
      """
    Then the call succeeded
    And the info field "page_size" equals 4

  Scenario: listing a file is bad input
    Given a jailed code surface
    And a file at "not-dir.txt" with content:
      """
      x
      """
    When I call coder::list-folder with payload:
      """
      {"path":"not-dir.txt"}
      """
    Then the call failed with code "C210"
