@engine @directory @directory_functions
Feature: directory::engine::functions::list and directory::engine::functions::info
  Thin enrichment layer over `engine::functions::list`. Lists every
  registered function with worker-name attribution; filters by search,
  prefix, worker; info endpoint folds in registered triggers and
  any matching how-to skill from skills_folder.

  Background:
    Given the iii engine is reachable

  # ── functions::list ────────────────────────────────────────────────

  Scenario: functions::list returns the directory::engine::* surface itself
    When I call directory::engine::functions::list with payload:
      """
      {"prefix": "directory::engine::"}
      """
    Then the directory functions list includes "directory::engine::functions::list"
    And  the directory functions list includes "directory::engine::functions::info"
    And  every directory functions list entry has function_id prefix "directory::engine::"

  Scenario: functions::list also surfaces the worker's own directory::skills::* registrations
    When I call directory::engine::functions::list with payload:
      """
      {"prefix": "directory::skills::"}
      """
    Then the directory functions list is non-empty
    And  the directory functions list includes "directory::skills::list"

  Scenario: search filter is case-insensitive across id and description
    When I call directory::engine::functions::list with payload:
      """
      {"search": "WORKERS::INFO"}
      """
    Then the directory functions list includes "directory::engine::workers::info"

  Scenario: every entry carries a worker_name when known
    When I call directory::engine::functions::list with payload:
      """
      {"prefix": "directory::engine::"}
      """
    Then every directory functions list entry has a non-null worker_name

  # ── functions::info ────────────────────────────────────────────────

  Scenario: functions::info returns full detail for a known directory function
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::functions::list"}
      """
    Then the directory function-info response has function_id "directory::engine::functions::list"
    And  the directory function-info response has a non-empty description
    And  the directory function-info response has a request_schema
    And  the directory function-info response has a response_schema

  Scenario: functions::info errors on an unknown function_id
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "nope::nope"}
      """
    Then the directory::engine::functions::info call fails with a message mentioning "not found"

  Scenario: functions::info rejects empty function_id
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "  "}
      """
    Then the directory::engine::functions::info call fails with a message mentioning "non-empty"

  # ── how-to skill discovery ─────────────────────────────────────────

  Scenario: functions::info surfaces a how-to skill declared via frontmatter functions array
    Given a how-to skill file at "guides/observe.md" with body:
      """
      ---
      type: how-to
      functions: ["directory::engine::functions::list"]
      ---
      # How to list functions

      Call directory::engine::functions::list with a prefix filter.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::functions::list"}
      """
    Then the directory function-info how_guide skill_id is "guides/observe"
    And  the directory function-info how_guide body contains "Call directory::engine::functions::list"

  Scenario: functions::info surfaces a how-to skill discovered via body grep
    Given a how-to skill file at "guides/grep.md" with body:
      """
      ---
      type: how-to
      ---
      # Discovery via body link

      To inspect a worker, see iii://fn/directory/engine/workers/info.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::workers::info"}
      """
    Then the directory function-info how_guide skill_id is "guides/grep"

  Scenario: functions::info ignores skills without type how-to frontmatter
    Given a skill file at "noise/plain.md" with body:
      """
      # plain skill

      Mentions directory::engine::functions::list but is not a how-to.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::functions::list"}
      """
    Then the directory function-info how_guide is absent

  # ── bundled how-to wiring via frontmatter function_id (singular) ───
  #
  # These two scenarios mimic the layout of the worker's bundled
  # `iii-directory/skills/directory/{engine,registry}/...md` files: a
  # how-to markdown with `function_id: <id>` in its frontmatter is
  # picked up by directory::engine::functions::info for the matching
  # `<id>`. We exercise one directory::engine::* function and one
  # directory::registry::* function to cover both surfaces.

  Scenario: functions::info surfaces a bundled how-to for a directory::engine::* function
    Given a how-to skill file at "directory/engine/functions/list.md" with body:
      """
      ---
      type: how-to
      function_id: directory::engine::functions::list
      title: How to list functions registered with the engine
      ---
      # When to use

      Use directory::engine::functions::list to discover what's callable.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::functions::list"}
      """
    Then the directory function-info how_guide skill_id is "directory/engine/functions/list"
    And  the directory function-info how_guide title is "How to list functions registered with the engine"
    And  the directory function-info how_guide body contains "discover what's callable"

  Scenario: functions::info surfaces a bundled how-to for a directory::registry::* function
    Given a how-to skill file at "directory/registry/workers/list.md" with body:
      """
      ---
      type: how-to
      function_id: directory::registry::workers::list
      title: List workers from the public registry
      ---
      # When to use

      Use directory::registry::workers::list to search the public registry.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::registry::workers::list"}
      """
    Then the directory function-info how_guide skill_id is "directory/registry/workers/list"
    And  the directory function-info how_guide title is "List workers from the public registry"
    And  the directory function-info how_guide body contains "search the public registry"

  # ── related_skills ─────────────────────────────────────────────────

  Scenario: functions::info surfaces related skills via literal function_id grep
    Given a how-to skill file at "guides/primary.md" with body:
      """
      ---
      type: how-to
      function_id: directory::engine::workers::info
      title: Inspect one connected worker
      ---
      # Inspect one connected worker

      Body.
      """
    And   a skill file at "notes/cross-ref.md" with body:
      """
      # Cross reference

      For details, also call directory::engine::workers::info from the agent loop.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::workers::info"}
      """
    Then the directory function-info how_guide skill_id is "guides/primary"
    And  the directory function-info related_skills includes skill_id "notes/cross-ref"
    And  the directory function-info related_skills does not include skill_id "guides/primary"
    And  every directory function-info related_skills entry has a non-empty title

  Scenario: functions::info surfaces related skills via iii://fn/ URI grep
    Given a skill file at "tour/worker-tour.md" with body:
      """
      # Worker tour

      See iii://fn/directory/engine/workers/info for the full schema.
      """
    When I call directory::engine::functions::info with payload:
      """
      {"function_id": "directory::engine::workers::info"}
      """
    Then the directory function-info related_skills includes skill_id "tour/worker-tour"
