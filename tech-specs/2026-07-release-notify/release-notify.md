# Worker release waves → Linear organization

**Date:** 2026-07-03
**Status:** Implemented (skill + docs); Linear pipeline setup is a manual
one-time step
**Tracking:** MOT-3861

## Problem

Worker releases (`<worker>/vX.Y.Z` tag push → `release.yml`) end at the
registry publish. Nothing records in Linear which issues shipped in which
release — the June GA wave was tracked as nine hand-made "Publish X v1.0.0"
tickets (MOT-3634…MOT-3652), a pattern this work retires.

## Goals

1. Group shipped issues by release **wave** in Linear — workers are released
   in batches, so one release object per wave, all shipped issues attached,
   and **one AI-written note with a section per worker** (name + version +
   changes).
2. Track the work under the **Harness 1.0** project (iii/MOT).

Slack notification was originally in scope but is already covered — the
Slack channels receive release notifications through the existing wiring, so
no CI change ships with this work. (An earlier revision designed a terminal
`announce` job in `release.yml` posting via `chat.postMessage`; see git
history of this file if that's ever wanted.)

## Decision

**Linear organization runs as a Claude Code skill (`/release-sync`), not CI.**
Releases are already driven from Claude Code sessions, and the Linear MCP
covers the whole surface: `save_release` (create the release), `save_issue`
`addReleases` (attach tickets), `save_release_note` (notes). AI judgment
beats a commit-scan regex: it can read PR bodies and attach work that never
got an `MOT-###` reference, and it can backfill or regroup historical
releases. No CI secrets or pipeline access key — the skill runs on the
user's interactive Linear MCP auth.

Rejected alternatives: `linear/linear-release-action` in CI (regex-only
grouping, per-tag not per-wave, needs an access key; the skill's catch-up
semantics cover the "forgot to run it" risk), and Linear view-subscription
Slack notifications (per-issue noise, wrong granularity).

## The skill: `.claude/skills/release-sync/SKILL.md`

Invoked as `/release-sync [tag ...]` from a release session, or any session
for catch-up/backfill. Full behavior lives in the skill itself; the shape:

1. **Scope.** With args: those tags. Without: every worker tag not recorded
   in the Workers pipeline — detected by grepping tags against existing
   release descriptions, which list their included tags (the idempotency
   record).
2. **Wave grouping.** Pending tags grouped by tag creation date (UTC day);
   same-day workers form one wave. More than 3 waves pending → show the
   plan and confirm before writing (backfill gate).
3. **Gather/attach.** Per tag: commit range = previous tag of the same
   worker → tag, path-filtered to `<worker>/`. Explicit `MOT-###` refs from
   commits and PR bodies, plus verified judgment attachments — never
   invented.
4. **Write.** One `save_release` per wave (name `Release YYYY-MM-DD`,
   version = date, stage completed, description = tag list) →
   `save_issue.addReleases` across the wave → **one** `save_release_note`
   with a `## <worker> vX.Y.Z` section per worker.
5. **Report.** Wave → release URL → tags → attached issues; zero-issue tags
   flagged.

Idempotent: wave lookup by version; re-runs update the release, append
newly-found tags to the description, regenerate the note; `addReleases` is
append-only.

## Linear one-time setup (admin, UI — pipelines have no creation API)

1. Settings → Releases → new pipeline **"Workers"**, type **continuous**,
   team **iii**. One shared pipeline; one release per wave, version = wave
   date. (Releases requires Business+ plan.)
2. No access key needed — the skill authenticates as the user via MCP.
3. Leave Linear's auto-generate-notes toggle **off**; the skill writes notes.
4. Optional, recommended: status automation "release completed → issue
   Done/Released".

## Rollout

- First no-args `/release-sync` run doubles as the backfill: recent tag
  history groups into historical waves (e.g. the day console/v1.2.0 +
  web/v1.2.1 + both providers shipped becomes one release). Review the
  created releases before treating it as routine.
- `docs/sops/release.md` gains an "Organize in Linear" section.
- Conventions doc in Linear:
  [Release workflow — workers](https://linear.app/motia/document/release-workflow-workers-a3240a17967f).

## Out of scope (deliberate)

- CI Slack announcements (already covered by existing channel wiring).
- Per-worker Linear pipelines or per-tag releases (waves are the unit).
- Headless/cron invocation of the skill (Linear MCP auth is interactive;
  catch-up semantics make periodic manual runs sufficient).
