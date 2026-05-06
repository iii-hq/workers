@pure @markdown
Feature: pure markdown, URI, and validation helpers
  Runs without the iii engine. Covers the helpers that back the
  `skills::resources-*` and prompts-normalize paths so the hot loop
  doesn't need an engine round-trip just to assert on them.

  # ── extract_title ────────────────────────────────────────────────────

  Scenario: extract_title picks up the first H1
    When I extract the title from:
      """
      # my skill

      body
      """
    Then the extracted title is "my skill"

  Scenario: extract_title ignores H2 headings
    When I extract the title from:
      """
      ## sub

      body
      """
    Then there is no extracted title

  # ── extract_description ─────────────────────────────────────────────

  Scenario: extract_description grabs the first paragraph
    When I extract the description from:
      """
      # title

      first paragraph here.

      second paragraph.
      """
    Then the extracted description is "first paragraph here."

  Scenario: extract_description skips subheadings before finding text
    When I extract the description from:
      """
      # title

      ## sub

      ### deeper

      finally text.
      """
    Then the extracted description is "finally text."

  Scenario: extract_description returns nothing for heading-only docs
    When I extract the description from:
      """
      # only a title
      """
    Then there is no extracted description

  Scenario: extract_description keeps a long first paragraph intact
    When I extract the description from:
      """
      # title

      This is intentionally a long first paragraph that runs past 140 characters so we can verify the description helper preserves the full author text without inserting an ellipsis or truncating midway.
      """
    Then the extracted description is "This is intentionally a long first paragraph that runs past 140 characters so we can verify the description helper preserves the full author text without inserting an ellipsis or truncating midway."

  # ── truncate_chars ───────────────────────────────────────────────────

  Scenario: truncate_chars keeps multibyte characters intact
    Given the string "áéíóú" repeated 50 times
    When I truncate it to 5 chars
    Then the truncated string has 8 chars
    And  the truncated string ends with ...

  # ── parse_uri ────────────────────────────────────────────────────────

  Scenario: parse_uri returns Index for iii://skills
    When I parse the URI "iii://skills"
    Then parse_uri returns the index shape

  Scenario: parse_uri returns Skill for iii://brain
    When I parse the URI "iii://brain"
    Then parse_uri returns a skill with id "brain"

  Scenario: parse_uri returns Skill for nested iii://parent/sub
    When I parse the URI "iii://parent/sub"
    Then parse_uri returns a skill with id "parent/sub"

  Scenario: parse_uri returns Skill for arbitrarily deep paths
    When I parse the URI "iii://a/b/c/d/e"
    Then parse_uri returns a skill with id "a/b/c/d/e"

  Scenario: parse_uri returns Section for iii://fn/{single}
    When I parse the URI "iii://fn/echo"
    Then parse_uri returns a section with function "echo"

  Scenario: parse_uri returns Section for iii://fn/{a}/{b} joining with ::
    When I parse the URI "iii://fn/scope/echo"
    Then parse_uri returns a section with function "scope::echo"

  Scenario: parse_uri returns Section for deep iii://fn/{...}
    When I parse the URI "iii://fn/resend/email/send"
    Then parse_uri returns a section with function "resend::email::send"

  Scenario: parse_uri rejects a missing scheme
    When I parse the URI "brain"
    Then parse_uri fails

  Scenario: parse_uri rejects iii://fn alone (no function path)
    When I parse the URI "iii://fn"
    Then parse_uri fails

  Scenario: parse_uri rejects empty segments anywhere
    When I parse the URI "iii://a//b"
    Then parse_uri fails

  # ── validate_id ──────────────────────────────────────────────────────

  Scenario: validate_id accepts kebab / underscore lowercase
    When I validate the skill id "my-skill-1"
    Then the skill id validation succeeds

  Scenario: validate_id rejects uppercase
    When I validate the skill id "UpperCase"
    Then the skill id validation fails

  Scenario: validate_id rejects 65-char ids
    When I validate a skill id of 65 lowercase letters
    Then the skill id validation fails

  # ── validate_name (prompts) ──────────────────────────────────────────

  Scenario: validate_name accepts kebab / underscore
    When I validate the prompt name "send-email"
    Then the prompt name validation succeeds

  Scenario: validate_name rejects :: in the name
    When I validate the prompt name "mcp::send"
    Then the prompt name validation fails

  Scenario: validate_arguments rejects duplicates
    When I validate a duplicate argument pair
    Then argument validation fails

  Scenario: validate_arguments rejects empty names
    When I validate an empty argument name
    Then argument validation fails

  # ── normalize_function_output (iii://skill/fn) ───────────────────────

  Scenario: normalize_function_output string -> markdown
    When I normalize a markdown string output
    Then the normalized mime is "text/markdown"
    And  the normalized text is "hello"

  Scenario: normalize_function_output {content} -> markdown
    When I normalize a {content} output
    Then the normalized mime is "text/markdown"
    And  the normalized text is "hi"

  Scenario: normalize_function_output other -> JSON
    When I normalize an arbitrary JSON output
    Then the normalized mime is "application/json"
    And  the normalized text is JSON containing "x"

  # ── normalize_prompt_output ─────────────────────────────────────────

  Scenario: normalize_prompt_output string -> single user message
    When I normalize a prompt output string
    Then the normalized messages length is 1

  Scenario: normalize_prompt_output messages -> pass-through
    When I normalize a prompt output with a messages array
    Then the normalized messages length is 2

  Scenario: normalize_prompt_output unsupported shape fails
    When I normalize a prompt output of unsupported shape
    Then prompt normalization fails

  # ── is_always_hidden ────────────────────────────────────────────────

  Scenario Outline: infra namespaces are hard-floored
    When I check the hard floor for "<function_id>"
    Then the function id is hard-floored

    Examples:
      | function_id          |
      | state::get           |
      | engine::workers      |
      | stream::publish      |
      | iii.on_foo           |
      | iii::internal        |
      | mcp::handler         |
      | a2a::send            |
      | skills::register     |
      | prompts::register    |

  Scenario Outline: ordinary namespaces are not hard-floored
    When I check the hard floor for "<function_id>"
    Then the function id is not hard-floored

    Examples:
      | function_id                   |
      | mem::observe                  |
      | brain::summarize    |
      | my-worker::my-fn              |
