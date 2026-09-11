@direct @lifecycle
Feature: adversarial coder lifecycle

  Scenario: create read update search list tree delete through the folded code surface
    Given a jailed code surface
    When I call coder::create-file with payload:
      """
      {"files":[{"path":"hello.txt","content":"hello\n","parents":true,"overwrite":false}]}
      """
    Then the result for "hello.txt" succeeded
    And the file "hello.txt" exists
    When I call coder::read-file with payload:
      """
      {"path":"hello.txt"}
      """
    Then the read content equals:
      """
      hello
      """
    When I call coder::update-file with payload:
      """
      {"files":[{"path":"hello.txt","ops":[{"op":"insert","at_line":2,"content":"greeting\n"},{"op":"replace","pattern":"hello","replacement":"hello, world","expect_matches":1}]}]}
      """
    Then the result for "hello.txt" succeeded
    And the file "hello.txt" contains "hello, world"
    When I call coder::search with payload:
      """
      {"query":"greeting","path":".","search_paths":false}
      """
    Then the search has a content match for "hello.txt" at line 2
    When I call coder::list-folder with payload:
      """
      {"path":"."}
      """
    Then the listing contains "hello.txt"
    When I call coder::tree with payload:
      """
      {"path":".","max_depth":1}
      """
    Then the tree contains "hello.txt"
    When I call coder::delete-file with payload:
      """
      {"paths":["hello.txt"]}
      """
    Then the result for "hello.txt" succeeded
    And the file "hello.txt" does not exist
