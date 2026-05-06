@engine @prompts_get
Feature: prompts::mcp-get (the internal RPC behind MCP prompts/get)
  `prompts::mcp-get` looks a prompt up by name, invokes its handler
  via `iii.trigger`, and normalizes the handler output into the
  `{ description, messages: [...] }` shape the MCP worker passes to
  clients. Unknown prompts error; infra-namespace handlers are
  rejected by the hard floor.

  Background:
    Given the iii engine is reachable

  Scenario: a string handler is wrapped as a single user message
    Given a prompt handler that returns a string
    When I call prompts::mcp-get on the scoped prompt
    Then the mcp-get call succeeds
    And  the messages array has length 1
    And  message 0 has role "user" and text "str body"
    And  the result description is "bdd prompt"

  Scenario: a {content} handler is wrapped as a single user message
    Given a prompt handler that returns a {content} object
    When I call prompts::mcp-get on the scoped prompt
    Then the mcp-get call succeeds
    And  the messages array has length 1
    And  message 0 has role "user" and text "obj body"

  Scenario: a {messages: [...]} handler is passed through unchanged
    Given a prompt handler that returns a {messages: [...]} object
    When I call prompts::mcp-get on the scoped prompt
    Then the mcp-get call succeeds
    And  the messages array has length 2
    And  message 0 has role "user" and text "m1"
    And  message 1 has role "assistant" and text "m2"

  Scenario: a handler returning an unsupported shape fails the call
    Given a prompt handler that returns an unsupported shape
    When I call prompts::mcp-get on the scoped prompt
    Then the mcp-get call fails
    And  the mcp-get error mentions "unsupported shape"

  Scenario: calling mcp-get on an unknown prompt returns a not-found error
    When I call prompts::mcp-get with an unknown name "no-such-prompt"
    Then the mcp-get call fails
    And  the mcp-get error mentions "not found"

  Scenario: a prompt pointing at an infra-namespace handler is rejected by the hard floor
    Given a prompt pointing at the infra function "state::set"
    When I call prompts::mcp-get on the scoped prompt
    Then the mcp-get call fails
    And  the mcp-get error mentions "internal namespace"
