`({ before, after }) → { added: [string], removed: [string] }` — tokenizes both inputs on whitespace and returns the word-level set difference.

## When to use

- Diffing two prompt drafts to track which terms changed between iterations.
- Detecting added or removed function ids in a stream of registry events.

## Notes

- The diff is set-based. Order, duplicates, and position are not preserved — `"a b a"` and `"a b"` are equivalent inputs.
- For a position-aware diff, use a dedicated diff worker; this one is for quick token-set comparison.
