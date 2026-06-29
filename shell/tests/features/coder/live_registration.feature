@live @registration
Feature: live coder registration

  Scenario: engine-dispatched coder functions are reachable through the shell worker
    Given a live jailed shell code surface
    When I call coder::info with payload:
      """
      {}
      """
    Then the call succeeded
    When I call coder::create-file with payload:
      """
      {"files":[{"path":"bdd-live-smoke.txt","content":"live\n","overwrite":true}]}
      """
    Then the call succeeded
    When I call coder::read-file with payload:
      """
      {"path":"bdd-live-smoke.txt"}
      """
    Then the read content contains "live"
    When I call coder::delete-file with payload:
      """
      {"paths":["bdd-live-smoke.txt"]}
      """
    Then the call succeeded
