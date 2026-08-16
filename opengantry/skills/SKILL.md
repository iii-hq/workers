---
name: opengantry
description: >-
  OpenGantry iii worker. Use when an iii project needs a verify gate on local
  workers/ plus gantry::verify before promote-class triggers. Not for session
  admission (use your IdP worker) and not for writing mission law.
---

# opengantry

Call `gantry::verify` with an absolute `repo_root`. The worker scans that repo's `workers/` directory for iii contracts, then runs the OpenGantry mission gate.

## When to use

- An iii project keeps local workers under `workers/` and wants those contracts checked on every verify.
- Promote-class functions on the governed port should stay blocked until verify passes.

## When not to use

- Session admission (`session::auth`) — that is the adopter's IdP worker.
- Writing `.gitagent/` law — Planner commits missions. The worker process does not.

## Functions

| Id | Role |
|----|------|
| `gantry::verify` | Scan local `workers/`, then `verifyMission` |
| `gantry::middleware` | Governed-port gate; promote-class needs a prior verify pass |
| `gantry::on-function-registration` | Block `gantry::*` squatting |
| `gantry::on-trigger-registration` | Block triggers bound to `gantry::` |
| `gantry::on-trigger-type-registration` | Always denied |

## Bootstrap (host only)

If `.gitagent` is missing, run `gantry init`, then:

```bash
node scripts/activate-opengantry-iii.mjs --bootstrap
```

That writes a default mission for a Planner commit. Do not run it from the sandboxed worker.
