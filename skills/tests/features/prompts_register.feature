@engine @prompts_register
Feature: prompts::register / unregister / list
  The prompts registry validates name, description, function_id, and
  arguments (no duplicate / empty names) before writing to the
  `prompts` scope. Re-registration overwrites; unregister is idempotent;
  list returns metadata sorted by name without dispatching handlers.

  Background:
    Given the iii engine is reachable

  # ── name validation ──────────────────────────────────────────────────

  Scenario: register rejects an empty name
    When I register a prompt with name "", description "x", function_id "test::handler"
    Then the prompts::register call fails
    And  the prompts::register error mentions "name must be non-empty"

  Scenario: register rejects an uppercase name
    When I register a prompt with name "BadName", description "x", function_id "test::handler"
    Then the prompts::register call fails
    And  the prompts::register error mentions "lowercase"

  Scenario: register rejects a name with a space
    When I register a prompt with name "bad name", description "x", function_id "test::handler"
    Then the prompts::register call fails

  Scenario: register rejects a name with :: like a function id
    When I register a prompt with name "mcp::register", description "x", function_id "test::handler"
    Then the prompts::register call fails

  # ── description / function_id validation ─────────────────────────────

  Scenario: register rejects a blank description
    When I register a prompt with name "ok", description "  ", function_id "test::handler"
    Then the prompts::register call fails
    And  the prompts::register error mentions "description must be non-empty"

  Scenario: register rejects a missing function_id
    When I register a prompt with name "ok", description "x", function_id ""
    Then the prompts::register call fails
    And  the prompts::register error mentions "function_id must be non-empty"

  # ── arguments validation ─────────────────────────────────────────────

  Scenario: register rejects duplicate argument names
    When I register a prompt with duplicate argument names
    Then the prompts::register call fails
    And  the prompts::register error mentions "duplicate argument name"

  Scenario: register rejects an empty argument name
    When I register a prompt with an empty argument name
    Then the prompts::register call fails
    And  the prompts::register error mentions "argument name must be non-empty"

  # ── round-trip ───────────────────────────────────────────────────────

  Scenario: register then list surfaces the scoped prompt
    When I register a scoped prompt "simple" pointing at "test::handler"
    Then the prompts::register call succeeds
    When I list prompts
    Then the scoped prompt appears in the listing
    And  each prompt listing entry carries arguments count, function_id, and registered_at
    And  the prompt listing is sorted by name

  Scenario: re-register overwrites and refreshes the timestamp
    When I register a scoped prompt "twice" pointing at "test::handler"
    And  I record the registered_at timestamp for the prompt
    And  I re-register the scoped prompt with description "refreshed"
    Then the prompts::register call succeeds
    And  the re-registered prompt timestamp is different from the first

  Scenario: unregister is idempotent
    When I register a scoped prompt "gone" pointing at "test::handler"
    And  I unregister the scoped prompt
    Then the last prompt unregister returned removed=true
    When I unregister the scoped prompt again
    Then the last prompt unregister returned removed=false
    When I list prompts
    Then the scoped prompt does not appear in the listing
