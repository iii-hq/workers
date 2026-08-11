---
name: reflex
description: Local tool router — propose the next iii function call for an objective with a calibrated confidence score, without spending a frontier-model generation.
---

# reflex

Use reflex when you need to know which iii function fits a natural-language
objective and do not want to spend a frontier-model generation deciding.
It runs a small on-device model over the live function catalog and returns a
proposed call with a calibrated confidence score. It never executes anything:
act on a proposal by triggering the proposed function yourself, through the
normal policy and approval path.

## When to reach for it

- Mechanical next-step decisions: "read this file", "list workers",
  "delete this state key", "fetch this URL" — one clear function, arguments
  derivable from the objective.
- Chains of such steps: execute the proposed call, pass the result back as
  `observation`, and route again for the next step.
- Calibration questions: "how reliable is local routing on this rig" is
  answered by `reflex::shadow::report`, not guessed.

## When not to

- Open-ended reasoning, multi-constraint planning, or anything where the
  right function is genuinely ambiguous — low confidence responses
  (`abstain` / `refuse`) mean exactly that; escalate to a frontier model.
- Composing a final text answer from gathered results — that is frontier
  work; reflex only picks calls.
- Anything requiring execution authority. reflex has none by design.

## Boundaries

- Trust the confidence score only at or above the threshold your own
  shadow report supports; below it, treat proposals as hints.
- The index follows the engine catalog automatically; `reflex::index::status`
  shows size and freshness, `reflex::index::refresh` forces a rebuild.
- Shadow mode observes turns fail-open and never mutates them; its log is
  local jsonl and doubles as fine-tune data.

## Functions

`reflex::route` (objective, optional observation), `reflex::index::status`,
`reflex::index::refresh`, `reflex::shadow::report`. Internal, not for direct
calls: `reflex::on-functions-change`, `reflex::shadow::pre-generate`,
`reflex::shadow::post-generate`.
