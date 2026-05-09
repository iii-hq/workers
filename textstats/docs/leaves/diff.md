<!-- partial-info: Per-skill body for `iii://<worker>/<leaf>` (where `<leaf>` is this file's stem). Author-chosen topical H1 (not the function id), `## When to use`, `## Notes`, and optional llm-only blocks. -->

# Comparing two text drafts

## When to use

- Diffing two prompt drafts to track which terms changed between iterations.
- Detecting added or removed function ids in a stream of registry events.

## Notes

- The diff is set-based. Order, duplicates, and position are not preserved — `"a b a"` and `"a b"` are equivalent inputs.
- For a position-aware diff, use a dedicated diff worker; this one is for quick token-set comparison.
