# Producing a transcript of a session branch

## When to use

- Producing a human-readable transcript of a conversation branch for review, sharing, or archiving.
- Generating a debug artefact during incident triage: write the HTML to disk or return it as a tool result.
- Driving a preview pane in a UI: the returned HTML can be embedded in an iframe or saved straight to a file.

## Notes

- The document is fully self-contained: no external CSS or JS, so it opens in any browser without further dependencies.
- Visual styling: user messages are cyan-tinted, assistant messages are white-on-dark, tool results are dim grey, thinking blocks are italic, custom messages are amber-tinted, branch summaries are orange, and compaction entries are green.
- `branch_leaf` is optional; without it the active path is the most-recently-appended thread.
- HTML special characters (`&`, `<`, `>`, `"`, `'`) in message content are escaped before insertion, so untrusted content cannot inject markup.
