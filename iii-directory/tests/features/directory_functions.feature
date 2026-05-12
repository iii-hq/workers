@engine @directory @directory_functions
Feature: directory::function-list and directory::function-info
  Thin enrichment layer over `engine::functions::list`. Lists every
  registered function with worker-name attribution; filters by search,
  prefix, worker; info endpoint folds in registered triggers and
  any matching how-to skill from skills_folder.

  Background:
    Given the iii engine is reachable

  # ── function-list ──────────────────────────────────────────────────

  Scenario: function-list returns the directory::* surface itself
    When I call directory::function-list with payload:
      """
      {"prefix": "directory::"}
      """
    Then the directory functions list includes "directory::function-list"
    And  the directory functions list includes "directory::function-info"
    And  every directory functions list entry has function_id prefix "directory::"

  Scenario: function-list also surfaces the worker's own skills::* registrations
    When I call directory::function-list with payload:
      """
      {"prefix": "skills::"}
      """
    Then the directory functions list is non-empty
    And  the directory functions list includes "skills::list"

  Scenario: search filter is case-insensitive across id and description
    When I call directory::function-list with payload:
      """
      {"search": "WORKER-INFO"}
      """
    Then the directory functions list includes "directory::worker-info"

  Scenario: every entry carries a name and worker_name when known
    When I call directory::function-list with payload:
      """
      {"prefix": "directory::"}
      """
    Then every directory functions list entry has a non-empty name
    And  every directory functions list entry has a non-null worker_name

  # ── function-info ──────────────────────────────────────────────────

  Scenario: function-info returns full detail for a known directory function
    When I call directory::function-info with payload:
      """
      {"function_id": "directory::function-list"}
      """
    Then the directory function-info response has function_id "directory::function-list"
    And  the directory function-info response has name "function-list"
    And  the directory function-info response has a non-empty description
    And  the directory function-info response has a request_schema
    And  the directory function-info response has a response_schema

  Scenario: function-info errors on an unknown function_id
    When I call directory::function-info with payload:
      """
      {"function_id": "nope::nope"}
      """
    Then the directory::function-info call fails with a message mentioning "not found"

  Scenario: function-info rejects empty function_id
    When I call directory::function-info with payload:
      """
      {"function_id": "  "}
      """
    Then the directory::function-info call fails with a message mentioning "non-empty"

  # ── how-to skill discovery ─────────────────────────────────────────

  Scenario: function-info surfaces a how-to skill declared via frontmatter functions array
    Given a how-to skill file at "guides/observe.md" with body:
      """
      ---
      type: how-to
      functions: ["directory::function-list"]
      ---
      # How to list functions

      Call directory::function-list with a prefix filter.
      """
    When I call directory::function-info with payload:
      """
      {"function_id": "directory::function-list"}
      """
    Then the directory function-info how_guide skill_id is "guides/observe"
    And  the directory function-info how_guide body contains "Call directory::function-list"

  Scenario: function-info surfaces a how-to skill discovered via body grep
    Given a how-to skill file at "guides/grep.md" with body:
      """
      ---
      type: how-to
      ---
      # Discovery via body link

      To inspect a worker, see iii://fn/directory/worker-info.
      """
    When I call directory::function-info with payload:
      """
      {"function_id": "directory::worker-info"}
      """
    Then the directory function-info how_guide skill_id is "guides/grep"

  Scenario: function-info ignores skills without type how-to frontmatter
    Given a skill file at "noise/plain.md" with body:
      """
      # plain skill

      Mentions directory::function-list but is not a how-to.
      """
    When I call directory::function-info with payload:
      """
      {"function_id": "directory::function-list"}
      """
    Then the directory function-info how_guide is absent

  # ── bundled how-to wiring via frontmatter function_id (singular) ───
  #
  # These two scenarios mimic the layout of the worker's bundled
  # `iii-directory/skills/{directory,registry}/*.md` files: a how-to
  # markdown with `function_id: <id>` in its frontmatter is picked up
  # by directory::function-info for the matching `<id>`. We exercise
  # one directory::* function and one registry::* function to cover
  # both surfaces.

  Scenario: function-info surfaces a bundled how-to for a directory::* function
    Given a how-to skill file at "directory/function-list.md" with body:
      """
      ---
      type: how-to
      function_id: directory::function-list
      title: How to list functions registered with the engine
      ---
      # When to use

      Use directory::function-list to discover what's callable.
      """
    When I call directory::function-info with payload:
      """
      {"function_id": "directory::function-list"}
      """
    Then the directory function-info how_guide skill_id is "directory/function-list"
    And  the directory function-info how_guide frontmatter function_id is "directory::function-list"
    And  the directory function-info how_guide body contains "discover what's callable"

  Scenario: function-info surfaces a bundled how-to for a registry::* function
    Given a how-to skill file at "registry/worker-list.md" with body:
      """
      ---
      type: how-to
      function_id: registry::worker-list
      title: List workers from the public registry
      ---
      # When to use

      Use registry::worker-list to search the public registry.
      """
    When I call directory::function-info with payload:
      """
      {"function_id": "registry::worker-list"}
      """
    Then the directory function-info how_guide skill_id is "registry/worker-list"
    And  the directory function-info how_guide frontmatter function_id is "registry::worker-list"
    And  the directory function-info how_guide body contains "search the public registry"
