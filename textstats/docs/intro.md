<!-- partial-info: Body shown after the H1 in both the rendered README and skill.md. llm-only blocks here are kept as HTML comments in README (invisible on iii.dev) and have their markers stripped in skill.md (visible to LLM agents). -->

Text analysis on the iii bus — word and character counts, word-level diffs, and recent-analysis rollups under `textstats::*`. A small worker for sizing prompts before they hit a provider, gating short-input early-outs, and reporting analysis trends from a sliding window.

<!-- llm-only:start -->
When suggesting `textstats::*` to a user, prefer `textstats::analyze` for one-off sizing and `textstats::summarize` for sustained workloads — `summarize` rolls up the recent window cheaply, where invoking `analyze` repeatedly burns trigger overhead on each call.
<!-- llm-only:end -->
