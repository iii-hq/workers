@engine @prompts
Feature: filesystem-backed prompts (directory::prompts::list / ::get / ::save)
  Prompts live at `<skills_folder>/<ns>/prompts/*.md` with YAML
  frontmatter declaring at least `description`. Both endpoints return
  plain JSON shapes — no MCP envelope, no role/messages wrapper — so
  the worker stays agnostic to MCP and any other adapter.

  Background:
    Given the iii engine is reachable

  # ── directory::prompts::list ───────────────────────────────────────

  Scenario: a fs prompt with frontmatter appears in directory::prompts::list
    Given a prompt file at "ns/prompts/open-pr.md" with content:
      """
      ---
      name: open-pr
      description: Open a GitHub pull request.
      ---
      Compose a PR body that follows the repository template.
      """
    When I list prompts
    Then the prompts listing has an entry "open-pr" with description "Open a GitHub pull request."
    And  the prompts listing entry "open-pr" has a non-empty modified_at

  Scenario: a fs prompt without frontmatter is skipped
    Given a prompt file at "ns/prompts/no-fm.md" with content:
      """
      # heading
      no frontmatter
      """
    When I list prompts
    Then the prompts listing does not include "no-fm"

  # ── directory::prompts::get ────────────────────────────────────────

  Scenario: directory::prompts::get returns the body, name, description, and modified_at
    Given a prompt file at "ns/prompts/static.md" with content:
      """
      ---
      description: Static prompt.
      ---
      The body content of the prompt.
      """
    When I get prompt "static"
    Then the prompt response has name "static"
    And  the prompt response has description "Static prompt."
    And  the prompt response body contains "The body content of the prompt."
    And  the prompt response has a non-empty modified_at

  Scenario: directory::prompts::get for an unknown name returns a not-found error
    When I get prompt "nope"
    Then the directory::prompts::get call fails with a message mentioning "not found"

  Scenario: directory::prompts::get rejects an empty name
    When I get prompt ""
    Then the directory::prompts::get call fails with a message mentioning "non-empty"

  Scenario: directory::prompts::get rejects a name with invalid characters
    When I get prompt "Bad/Name"
    Then the directory::prompts::get call fails with a message mentioning "lowercase"

  # ── directory::prompts::save ───────────────────────────────────────

  Scenario: directory::prompts::save creates a prompt readable via get
    When I save prompt "bdd-created" with strategy "override" and body:
      """
      Reply only in haiku.
      """
    And I get prompt "bdd-created"
    Then the prompt response has name "bdd-created"
    And  the prompt response has strategy "override"
    And  the prompt response body contains "Reply only in haiku."

  Scenario: directory::prompts::save is create-only
    When I save prompt "bdd-dupe" with strategy "enrich" and body:
      """
      First body.
      """
    And I save prompt "bdd-dupe" with strategy "enrich" and body:
      """
      Second body.
      """
    Then the directory::prompts::save call fails with a message mentioning "already exists"
