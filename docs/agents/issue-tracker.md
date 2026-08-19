# Issue tracker: Linear

Issues and specs for this repo live in Linear, on the **iii team** (ticket prefix `MOT-###`). Use the Linear MCP tools (`mcp__linear-server__*`) for all operations; there is no CLI.

## Conventions

Follow the repo's `AGENTS.md` (Commit & PR conventions, Linear ticket conventions). In particular:

- Ticket **title and opening paragraph are user-facing**: plain product language, no jargon — they feed release notes. Implementation notes go after, under a `## Technical details` heading.
- Don't flood Linear: correlated PRs share one ticket. If a piece genuinely needs its own tracking, create a **sub-issue** under the main ticket, not a new top-level ticket.
- Create tickets on the iii team in the matching project. Worker-specific tickets get the matching `worker:<name>` label.
- Commits and PR titles carry the ticket prefix `(MOT-###)`; the PR body carries `Fixes MOT-###` (or `Refs MOT-###` for secondary tickets).

## Operations

- **Create an issue**: `save_issue` with `team: "iii"`, a user-facing title/description, and labels.
- **Read an issue**: `get_issue` with the `MOT-###` identifier; `list_comments` for the discussion.
- **List issues**: `list_issues` filtered by team/state/label/project.
- **Comment**: `save_comment` on the issue.
- **Apply / remove labels**: `save_issue` with the updated `labels` list. Create missing labels with `create_issue_label` on the iii team (see `docs/agents/triage-labels.md`).
- **Close**: `save_issue` moving the issue to Done (or Canceled for wontfix), with a closing comment.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external GitHub PRs as feature requests; `/triage` reads this flag.)_

## When a skill says "publish to the issue tracker"

Create a Linear issue on the iii team.

## When a skill says "fetch the relevant ticket"

`get_issue` with the `MOT-###` identifier, plus `list_comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single Linear issue with **sub-issues** as tickets.

- **Map**: an issue holding the Notes / Decisions-so-far / Fog body, labelled `wayfinder:map` (create the label on first use).
- **Child ticket**: a sub-issue under the map (`save_issue` with `parent`). Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, assign to the driving dev.
- **Blocking**: Linear's native **blocked by** relations. A ticket is unblocked when every blocker is Done/Canceled.
- **Frontier query**: `list_issues` for the map's open sub-issues; drop any with an open blocker or an assignee; first in map order wins.
- **Claim**: assign the issue to yourself — the session's first write.
- **Resolve**: post the answer as a comment, move the issue to Done, then append a context pointer to the map's Decisions-so-far.
