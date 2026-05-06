@engine @skills_nested
Feature: nested skills (multi-level state-backed bodies + path-mapped sections)
  Skills can be registered at any depth via slashed ids (`a/b/c/...`).
  Each level is an independent state row; parents don't have to exist
  first. The auto-rendered `iii://skills` index indents every entry by
  `2 * depth` spaces. The function-trigger URI shape uses `fn` as a
  fixed first-segment marker — `iii://fn/a/b/c` triggers `a::b::c`.
  The `fn` literal is reserved as the first segment of any registered
  id so `iii://fn/...` is never ambiguous with a stored body.

  Background:
    Given the iii engine is reachable

  # ── deep state-backed bodies ─────────────────────────────────────────

  Scenario: a single-level sub-skill resolves to its own body
    Given a nested skill at "parent" with body "# parent\nrouter\n"
    And   a nested skill at "parent/sub" with body "# child\nleaf body\n"
    When I read the nested skill at "parent/sub"
    Then the nested read mime is "text/markdown"
    And  the nested read text contains "leaf body"

  Scenario: deep traversal — three independent bodies at depths 1/2/3
    Given a nested skill at "deep" with body "# deep\ntop\n"
    And   a nested skill at "deep/middle" with body "# deep/middle\nmid\n"
    And   a nested skill at "deep/middle/leaf" with body "# deep/middle/leaf\nleaf\n"
    When I read the nested skill at "deep"
    Then the nested read text contains "top"
    When I read the nested skill at "deep/middle"
    Then the nested read text contains "mid"
    When I read the nested skill at "deep/middle/leaf"
    Then the nested read text contains "leaf"

  Scenario: an orphan grandchild is still readable when the parents are absent
    Given a nested skill at "orphan/middle/grand" with body "# grand\nleaf only\n"
    When I read the nested skill at "orphan/middle/grand"
    Then the nested read text contains "leaf only"

  Scenario: skills::list returns nested entries sorted lexicographically
    Given a nested skill at "z-tree" with body "# z\n"
    And   a nested skill at "z-tree/a" with body "# a\n"
    And   a nested skill at "z-tree/b" with body "# b\n"
    When I list the seeded nested ids
    Then the listing is sorted with parent before children for "z-tree"

  Scenario: the iii://skills index indents nested entries by depth
    Given a nested skill at "tree-root" with body "# Tree root\nTop level body.\n"
    And   a nested skill at "tree-root/leaf" with body "# Tree leaf\nA child entry.\n"
    When I read iii://skills via fetch
    Then the index has the seeded entry "tree-root" indented by 0 spaces
    And  the index has the seeded entry "tree-root/leaf" indented by 2 spaces

  # ── reserved `fn` first segment ──────────────────────────────────────

  Scenario: registering id "fn" is rejected with a "reserved" hint
    When I register a skill with id "fn" and body "# nope\n"
    Then the skills::register call fails
    And  the skills::register error mentions "reserved"

  Scenario: registering id starting with "fn/" is rejected
    When I register a skill with id "fn/anything" and body "# nope\n"
    Then the skills::register call fails
    And  the skills::register error mentions "reserved"

  Scenario: registering an id with "fn" at a non-first segment succeeds
    When I register a skill with id "docs/fn-reference" and body "# fn reference\nallowed\n"
    Then the skills::register call succeeds

  # ── path-mapped section URIs ─────────────────────────────────────────

  Scenario: iii://fn/{a}/{b} triggers a::b
    Given a deep section function whose id has 2 scope segments
    When I read the section URI for the seeded deep function
    Then the nested read mime is "text/markdown"
    And  the nested read text contains "deep-section"

  Scenario: iii://fn/{a}/{b}/{c} triggers a::b::c
    Given a deep section function whose id has 3 scope segments
    When I read the section URI for the seeded deep function
    Then the nested read mime is "text/markdown"
    And  the nested read text contains "triple-section"

  # ── skill::fetch composition over deep URIs ──────────────────────────

  Scenario: skill::fetch concatenates bodies across depths
    Given a nested skill at "fetched" with body "# fetched\nALPHA-BODY\n"
    And   a nested skill at "fetched/sub" with body "# fetched/sub\nBETA-BODY\n"
    And   a nested skill at "fetched/sub/leaf" with body "# fetched/sub/leaf\nGAMMA-BODY\n"
    When I fetch the seeded nested URIs at depths 1, 2, 3
    Then the fetched text contains "ALPHA-BODY"
    And  the fetched text contains "BETA-BODY"
    And  the fetched text contains "GAMMA-BODY"
    And  the fetched text has 3 sections joined by "---"
