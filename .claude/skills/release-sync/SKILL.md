---
name: release-sync
description: Organize worker release tags into Linear release waves — one Linear Release plus one combined note per same-day batch of worker releases, with shipped MOT issues attached. Use after cutting worker releases, or any time to catch up or backfill. Trigger: /release-sync [tag ...]
---

# Release sync — worker tags → Linear release waves

One **wave** = all `<worker>/vX.Y.Z` tags created on the same UTC day.
Each wave maps to **one** release in the Linear **Workers** pipeline
(version = `YYYY-MM-DD`), with every shipped `MOT-###` issue attached and
**one** release note holding a `## <worker> vX.Y.Z` section per worker.

## Prerequisites

- Linear MCP tools (the claude.ai Linear connector).
- The **Workers** release pipeline must exist (`list_release_pipelines`).
  If it doesn't, stop and tell the user to create it in Linear:
  Settings → Releases → new pipeline **Workers**, type *continuous*,
  team *iii* (Releases requires Business+ plan).

## 1. Collect pending tags

```bash
git fetch --tags origin
git for-each-ref 'refs/tags/*/v*' --sort=creatordate \
  --format='%(refname:short) %(creatordate:short)'
```

- With arguments: sync exactly those tags.
- Without arguments: a tag is already synced iff it appears in the
  **description of an existing release** in the Workers pipeline
  (`list_releases`, read the descriptions — each lists its included tags).
  Pending = every worker tag not found there.
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
  and verify it matches before attaching. Never invent or guess
  identifiers; when unsure, leave it off and flag it in the report.

## 4. Write to Linear (per wave)

1. Look up the wave release by version (`list_releases`, pipeline
   *Workers*, version = the date). Update it if it exists, else
   `save_release`: name `Release YYYY-MM-DD`, version `YYYY-MM-DD`,
   stage `completed`, description listing one tag per line — this list is
   the idempotency record, always keep it complete.
2. Attach issues with `save_issue` + `addReleases` (append-only; never
   `setReleases`, never remove existing attachments).
3. One note per wave (`save_release_note`, create or update): a
   `## <worker> vX.Y.Z` section per tag with 2–5 bullets — what shipped,
   why it matters, breaking changes called out as **Breaking:**. Plain
   changelog register; no internal workflow chatter.

## 5. Report

Table: wave date → Linear release URL → tags → attached issues. Flag tags
where no issues were found and any judgment attachments made.

## Rules

- Never change issue status (release automations may own that), never
  create issues, never remove release attachments.
- Re-runs must converge: a late same-day tag merges into the existing
  wave and the note is regenerated to include it.
