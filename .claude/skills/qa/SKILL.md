---
name: qa
description: Automated black-box QA of this branch through the UI, as a brand-new user. Brings up a throwaway first-run stack built from the branch, turns every change into test charters, hands them to independent QA tester agents that drive the real app (never the code), and returns one report of broken functionality, bad UX, UI issues and anything else missed. Use when asked to "QA this", "test the changes as a user", "run QA before the PR", or "find what we missed". Reports only; fixing is the `tester` skill's job.
---

# QA

The developer who built a change is the worst person to test it. They
already know what the code does. So this skill keeps the two roles apart:

- **You (the coordinator)** prepare a clean new-user stack, write the
  charters from what changed, start the testers, and merge their reports.
- **QA tester agents** test the running app in a browser. Each one is a fresh
  agent that follows `tester-agent.md` in this folder. They get the
  charters, never the code or your opinions about it.

Nothing here edits source. Findings go to a report. Fix them later with the
`tester` skill, which does repro, fix, test, and before/after.

## 1. A brand-new user's stack

```sh
S=.claude/skills/qa/scripts
$S/stack.sh reset && $S/stack.sh up   # template/ + this branch's workers, empty data, ns <worktree>-qa
tail -f .context/qa-stack/compose.log # wait for "Containers Running"
$S/web.sh                             # builds changed worker UIs, vite on http://localhost:5197/
```

- `stack.sh` renders the repo's `template/` (what a new user downloads) into
  `.context/qa-stack/` and points every worker at this branch. It runs on
  engine 49164 and console 3143, so the person's own stack on 49134/3113 is
  never touched. With a warm `CARGO_TARGET_DIR` it is up in about 30 s; a
  cold build takes minutes.
- `QA_SKIP=sandbox-libkrun,computer` leaves out the VM image (a large
  download) when the change has nothing to do with computer sessions. Say in
  the report that it was skipped.
- `reset` wipes `data/`, so the next run is a first run again (the setup
  wizard opens by itself). Reset between full QA rounds.
- **Model access.** A new user has no provider. Ask the person once whether
  to connect one: an API key exported in `workers-dev.env` (keys go to
  llm-router), or a local Codex/Claude Code login. Without one, the testers
  cover everything that doesn't need a reply and list the rest as blocked.
  Never paste a key into a report, a screenshot or a brief.
- Need two testers at once? Every tester on one stack shares the same
  workspace layout, so give each its own stack: `QA_NAME=qa2
  QA_ENGINE_PORT=49174 QA_CONSOLE_PORT=3153 QA_WEB_PORT=5198` on both
  scripts.

## 2. Charters from the changes

```sh
$S/changes.sh        # commit subjects + areas touched;  -v adds bodies
```

Rewrite each change as a user-facing **charter**: "As a user I can …
/ I should see …". Each charter gets a claim and where to find it, not how
it is built. One commit can give several charters. Mark each one P0 (core
flow) to P2.

Always add the **baseline journeys**. A change anywhere can break them:

| # | Journey |
| --- | --- |
| J1 | First run: every wizard step, Skip, close mid-way, reload mid-way, reopen it |
| J2 | Connect a model (or record "blocked") |
| J3 | First chat: send, stream, stop, queue while streaming, edit the queued message, attach and drop a file, model, agent and folder pickers |
| J4 | The agent asks a question: answer, skip, dismiss; two chats open; reload; a second tab |
| J5 | Chats: new, rename, pin, search, delete, import, collapse the sidebar, the header title |
| J6 | Workspace: split, open every page, close, reorder, workspace tabs, ⌘K, the shortcuts dialog |
| J7 | Each worker page: one real action (state, cron, sentinel, kanban, database, browser, ide, directory, voice, traces, workers, functions, observers) |
| J8 | Settings: theme, each section, a worker's settings form (change it, reload, it stuck) |
| J9 | Phone 390×844 for J3, J5 and J6 |
| J10 | Keyboard only for J3 and J6 |

Write the brief to `.context/qa/<YYYY-MM-DD-HHMM>/brief.md`: the QA URL, the
date and branch, the charters, and the journeys for each tester.

## 3. Run the testers

Split the work into 2–4 groups by area (for example: chat and questions;
workspace, palette and settings; worker pages; first run, phone and
keyboard). Spawn one fresh agent per group with the Agent tool
(`subagent_type: "general-purpose"`, never a fork of yourself, because a
fork inherits everything you know about the code). Run them in parallel
only when each one has its own stack (§1); otherwise run them one after
another. Each prompt contains only:

- "You are a QA tester. Read `.claude/skills/qa/tester-agent.md` and follow
  it exactly. Your brief:";
- the QA URL, and the output folder `.context/qa/<run>/<group>/`;
- that group's charters and journeys, copied from the brief;
- whether a model is connected;
- when the person wants a video: "Video: REQUIRED", with clip names prefixed
  by the group letter (`A-01-…`). The testers record with `launch({ video })`
  and `caption()` from the tester helpers.

Don't add hints about the implementation, suspected bugs, or file names. A
tester that is told where the bug is will find it there and stop looking
anywhere else.

## 4. Merge and report

Read every `<group>/report.md` (a tester whose file write was refused returns the report as text; save it there). Before merging, look at each finding's
screenshot yourself and reproduce any P0 or P1 once with its steps. Drop a
finding only if it does not reproduce, and say that you dropped it. Merge
duplicates and give the findings new IDs. Write
`.context/qa/<run>/report.md` with the same six sections the testers use
(Functionality not working, UX is bad, UI issues, Other, Passed, Not
covered), with a table at the top:

| ID | Sev | Section | Title | Change | Evidence |

Show the person the table, the worst three findings with their screenshots
inline, and the "Not covered" list. Ask which ones to fix. Fixing goes
through the `tester` skill, one bug at a time, each with before/after.

**Video.** Combine the clips into one film with title cards and a results
card, then open it for the person:

```sh
cp $S/video.mjs ade/web/node_modules/.cache/qa-video.mjs
node ade/web/node_modules/.cache/qa-video.mjs .context/qa/<run> "chat-harness=Chat and harness" "worker-pages=Worker pages" …
open .context/qa/<run>/qa-video.mp4
```

When done, `stack.sh down` and stop the vite on 5197 (`lsof -ti
tcp:5197 | xargs kill`). Leave `.context/qa/` in place, because the next
round compares against it.
