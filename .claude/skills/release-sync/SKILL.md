---
name: release-sync
description: Organize worker release tags into Linear release waves — one team document plus one release/<date> label per same-day batch of worker releases, with shipped MOT issues labeled and linked. Use after cutting worker releases, or any time to catch up or backfill. Trigger: /release-sync [tag ...]
---

# Release sync — worker tags → Linear release waves

One **wave** = all `<worker>/vX.Y.Z` tags created on the same UTC day.
Each wave maps to one **Linear document** on team **iii** titled
`Release YYYY-MM-DD` — the combined release note, one `## <worker> vX.Y.Z`
section per worker — plus a child label `YYYY-MM-DD` under the team label
group **release**, applied to every shipped issue. (This is the free-plan
model; Linear's native Releases feature requires Business+.)

## Prerequisites

- Linear MCP tools (the claude.ai Linear connector).
- Team label group **release** on team iii (create once with
  `create_issue_label`, `isGroup: true`, if missing).

## 1. Collect pending tags

```bash
git fetch --tags origin
git for-each-ref 'refs/tags/*/v*' --sort=creatordate \
  --format='%(refname:short) %(creatordate:short)'
```

- Only consider tags that still exist on origin (`git ls-remote --tags
  origin` on doubt) — rolled-back local tags don't count.
- With arguments: sync exactly those tags.
- Without arguments: a tag is already synced iff it appears in the
  **header tag list of an existing wave document** (`list_documents`,
  query `Release`, team iii — each doc's first line lists its tags; that
  list is the idempotency record). Pending = every worker tag not found.
- Skip `-dry-run.` tags. Include prereleases (`-rc.N`, `-beta.N`) and label
  them in the note.

## 2. Group into waves

Group pending tags by tag creation date (UTC day). If more than 3 waves
would be written (typical of a first backfill), show the plan
(date → tags) and get user confirmation before writing anything.

## 3. Gather changes per tag

- Previous tag of the same worker:
  `git tag -l '<worker>/v*' --sort=-v:refname` → the entry after the
  current one.
- Commits: `git log --format='%h %s%n%b' <prev>..<tag> -- <worker>/`
  (first release of a worker: `git log <tag> -- <worker>/`).
- Extract `MOT-\d+` from subjects and bodies. For PR references `(#N)`
  with no MOT id, `gh pr view N --json title,body` and scan those too.
- Judgment attachments: if a change clearly implements a known issue that
  was never referenced, find it via `list_issues` (title terms, team iii)
  and read the issue to verify it matches before labeling. Never invent
  or guess identifiers; when unsure, leave it off and flag it in the
  report.

## 4. Write to Linear (per wave)

1. Label: child label `YYYY-MM-DD` under group **release** (team iii,
   `create_issue_label` with `parent: release`; skip if it exists).
2. Label every verified issue: read the issue's current labels first and
   pass the union to `save_issue` — never drop existing labels.
3. Document: `save_document` on team iii titled `Release YYYY-MM-DD`
   (update the existing one if present — look it up via `list_documents`).
   First line: the full tag list (idempotency record — always complete)
   and the wave label name. Body: a `## <worker> vX.Y.Z` section per tag
   (collapse multiple same-day tags of one worker into
   `## <worker> vA → vB`), 2–5 bullets each — what shipped, why it
   matters, breaking changes called out as **Breaking:**, shipped issues
   linked by URL. Plain changelog register; no internal workflow chatter.

## 5. Report

Table: wave date → document URL → tags → labeled issues. Flag tags where
no issues were found and any judgment attachments made.

## Rules

- Never change issue status, never create issues, never remove existing
  labels from an issue.
- Re-runs must converge: a late same-day tag merges into the existing
  wave document and the note is regenerated to include it.
