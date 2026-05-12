@engine @download @download_repo
Feature: skills::download repo= source (git clone --depth 1)
  Pulls a single subfolder under `skills/<skill>/` from a git repo
  into `<skills_folder>/<skill>/`. Re-pulls overwrite file-by-file.
  Markdown files that include a `prompts/` segment are recorded under
  `prompts_written` so the on-change fanout fires `prompts::on-change`
  selectively.

  Background:
    Given the iii engine is reachable

  Scenario: downloads all files from skills/<skill>/ in the repo
    Given a local git repo with a folder skills/frontend-design/ containing:
      | path                     | body                              |
      | index.md                 | # frontend-design\nrouter\n      |
      | components.md            | # components\nleaf\n             |
      | layouts/grid.md          | # grid\nnested\n                 |
    When I trigger skills::download with repo=local skill="frontend-design"
    Then the skills::download call succeeds
    And  the file "frontend-design/index.md" exists in skills_folder
    And  the file "frontend-design/components.md" exists in skills_folder
    And  the file "frontend-design/layouts/grid.md" exists in skills_folder
    And  the download response namespace is "frontend-design"
    And  the download skills_written count is 3

  Scenario: prompts inside skills/<skill>/prompts/ count as prompts
    Given a local git repo with a folder skills/with-prompts/ containing:
      | path                  | body                                          |
      | index.md              | # with-prompts\nrouter\n                     |
      | prompts/say-hello.md  | ---\ndescription: say hi.\n---\nHello there. |
    When I trigger skills::download with repo=local skill="with-prompts"
    Then the skills::download call succeeds
    And  the download skills_written count is 1
    And  the download prompts_written count is 1

  Scenario: re-pulling overwrites files inside the namespace
    Given a local git repo with a folder skills/foo/ containing:
      | path         | body                |
      | index.md     | # foo v1\noriginal\n |
    When I trigger skills::download with repo=local skill="foo"
    Then the skills::download call succeeds
    When I update the local repo file "skills/foo/index.md" to body "# foo v2\nupdated\n"
    And  I trigger skills::download with repo=local skill="foo"
    Then the skills::download call succeeds
    And  the file "foo/index.md" in skills_folder contains "v2"

  Scenario: download fails when the requested skill folder is missing
    Given a local git repo with a folder skills/something-else/ containing:
      | path     | body          |
      | index.md | # other\nx\n |
    When I trigger skills::download with repo=local skill="missing-skill"
    Then the skills::download call fails with a message mentioning "skills/missing-skill"

  Scenario: download is rejected when both repo and worker are provided
    When I trigger skills::download with both repo="https://x" and worker="resend" and tag="latest"
    Then the skills::download call fails with a message mentioning "not both"
