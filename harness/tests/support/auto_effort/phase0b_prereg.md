# Auto-effort phase 0b: preregistration

Written 2026-10-19, committed before any fresh prompt is labelled or judged.

## Why a phase 0b

Phase 0's G1 failed for TypeSafe on too-low picks (3.7% against a 3% bar). All four misses came from the fallback:
when the judge was unsure (confidence below 0.5), it kept the session's current level, and those sessions ran at an
unknown level or at the provider default (minimal/low). TypeSafe's confident picks had no too-low miss. Phase 0's
corpus is spent, so a fallback change has to be confirmed on prompts nobody has seen.

## Exploration (phase 0 corpus, not decisive)

TypeSafe re-run with raw choices recorded, graded on top plus effortless-holdout (109 rows):

| policy | right | too low | G1 |
|---|---|---|---|
| keep (phase 0) | 92.7% | 3.7% | FAIL |
| floor (unsure: never below medium) | 91.7% | 1.8% | PASS |
| argmax (always the judge's pick) | 92.7% | 1.8% | PASS |

argmax was best on every slice (child: 96.7% right, 0% too low) and depends least on the session's current level.

## Hypothesis

TypeSafe with the **argmax** fallback policy passes G1 on fresh prompts.

## Data

- Source: Claude Code transcripts, `~/.claude/projects/*/*.jsonl`, leaving out the session that ran this work
  (`df8c786c-cb75-4497-b680-fd604fe1b3f5`).
- Rows: `cc_extract.py --n 300 --per-session 5 --seed 20261019`, which gives typed human prompts only. Prompts that look like secrets are dropped, exact repeats are dropped, and each row's level and model come from the reply that answered it.
- Labels: `label.py`, the same rubric and Opus 5.5 labellers as phase 0. If fewer than 70% of rows agree, stop and report.

## Test

- Judge: the TypeSafe arm only. Haiku is dropped: its phase 0 p95 of 3.8 s fails the latency bar regardless.
- Policy: **argmax** is primary and decides. floor and keep are reported for information only.
- Gate: G1 on slice `cc` only: right ≥ 90%, too low ≤ 3%, p95 ≤ 1500 ms (`analyze.py --policy argmax --g1 cc`).

## Decision

- PASS: continue with phase 0's G2 (cache probe) and G3 (A/B), with policy argmax.
- FAIL: Phase 1 is not built, and no further policy is tried on this data.

## Known limits

- The prompts come from Claude Code, not the harness. The task (picking an agentic coding assistant's effort per message) is the same, but the mix of prompts differs.
- Claude Code sessions ran mostly at xhigh. Under argmax, current levels only matter for the fallback rows (errors and short follow-ups).
- The labellers are Opus 5.5, so the gold labels share that model's biases.
