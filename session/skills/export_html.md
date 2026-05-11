# session-tree::export_html

Render the active path of a session as a self-contained HTML document and return it
as a string.

`({ session_id, branch_leaf? }) → { html }` — walks the active path from root to
`branch_leaf` (or the most-recently-appended entry if omitted), renders each entry
as a styled `<div>`, and wraps the output in a complete `<!DOCTYPE html>` document
with inline CSS. All HTML special characters in message content are escaped.

## When to use

- Producing a human-readable transcript of a conversation branch for review, sharing,
  or archiving.
- Generating a debug artefact when diagnosing an agent run: write the HTML to disk or
  return it as a tool result.
- Building a preview pane in a UI: the returned HTML can be embedded in an iframe or
  written directly to a file.

## Notes

- The document is fully self-contained (no external CSS or JS dependencies); it can
  be saved as a `.html` file and opened in any browser.
- Visual styling: user messages are cyan-tinted, assistant messages are
  white-on-dark, tool results are dim grey, thinking blocks are italic, custom
  messages are amber-tinted, branch summaries are orange, and compaction entries are
  green.
- `branch_leaf` is optional; omit it to export the current active path (the main
  thread as recorded).
- Only entries on the active path are rendered; sibling branches are excluded.
- HTML special characters (`&`, `<`, `>`, `"`, `'`) in message content are escaped
  before insertion, preventing XSS in browser rendering.
