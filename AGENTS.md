# Agent guide

Conventions for AI coding agents (and humans) working in this repo.

## Commit & PR conventions

- No AI/assistant references in commit messages or PR descriptions: no
  session trailers, no tool URLs, no attribution or co-author lines.
- No internal-workflow references in commits/PRs/changelogs: no plan,
  sub-task, skill, or agent mentions. Describe the change itself — what it
  does and why.
- Branch names: `feat/<slug>`, `fix/<slug>`, `chore/…`, `refactor/…`,
  `docs/…` — never Linear's generated `user/mot-####` branch names.
- Commit messages and PR titles carry the Linear ticket as a prefix:
  `(MOT-##) <type>: <description>` (scoped types fine:
  `(MOT-123) feat(console): …`). PRs squash-merge, so the PR title is what
  lands on main and MUST carry the prefix — the `PR Linear Check` workflow
  fails without it.
- PR body adds `Fixes MOT-###` when merging should transition the ticket
  (Linear state automation); `Refs MOT-###` for secondary tickets. No
  ticket yet? Create one on the iii team in the matching project first.
- Genuinely ticketless work (version bumps, typo/CI-only) drops the prefix
  and the PR gets the `no-ticket` label instead.

## Linear ticket conventions

- Ticket title and the opening paragraph are user-facing: plain product
  language, no jargon — they feed release notes. Implementation notes go
  after, under a `## Technical details` heading.
- Don't flood Linear: correlated PRs share one ticket (`Fixes` on the main
  PR, `Refs MOT-###` on the others). If a piece genuinely needs its own
  tracking, create a sub-issue under the main ticket, not a new top-level
  ticket.
