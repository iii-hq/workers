# Composer: Enter sends, Shift+Enter breaks the line

## Goal

Make the chat composer (`ade/web/src/components/chat`) behave like every
other chat editor: **Enter sends the message, Shift+Enter inserts a new
line**. Today only `Mod+Enter` (⌘↵ on a Mac, Ctrl+Enter elsewhere) sends and
every other Enter adds a line, which surprises users.

## Approach (agreed: option B)

Enter is contextual so markdown structure stays editable:

| Where the caret is | Enter | Shift+Enter | Mod+Enter |
| --- | --- | --- | --- |
| Prose (paragraph) | **send** | line break | send |
| Line holding only ```` ``` ```` (optional lang) | open a code block (unchanged) | line break | send |
| Inside a code block | next line; a second blank line at the end leaves the block (unchanged) | next line | send |
| Inside a list item | next item; Enter on an empty item leaves the list (unchanged, ListPlugin) | line break (RichTextPlugin) | send |
| Typeahead menu open | swallowed (unchanged) | swallowed | swallowed |
| Composer disabled | nothing | nothing | nothing |

Rejected: option A (Enter always sends, even inside code/lists) — makes
editing a code block or a list awkward; option C (a user preference) — out
of scope, the user asked for the standard behaviour.

## Components

- `ade/web/src/components/chat/LexicalShell.tsx`
  - `SEND_BINDING` becomes `'Enter'` (what the send button advertises).
  - New `SEND_ANYWHERE_BINDING = 'Mod+Enter'`: sends from any position,
    structure or not.
  - `ComposerEnterPlugin` order: menu open → swallow; `Mod+Enter` → submit;
    not a range selection → fall through; code block → `$insertCodeLine`;
    list item → fall through to ListPlugin; paragraph: fence-only line →
    open code block; Shift held → fall through (line break); otherwise →
    `preventDefault` + `onSubmit`.
  - Doc comments rewritten to describe the new rule.
- `ade/web/src/components/chat/Composer.tsx`
  - `sendShortcutLabel()` keeps reading `SEND_BINDING`, so the send button
    title becomes `send message (↵)` with no code change beyond the comment.
- `ade/web/src/components/chat/LexicalShell.test.tsx`
  - `composer keyboard submission` suite rewritten for the table above.

## Data changes

None.

## Error handling

No new failure modes: a fall-through (`return false`) keeps Lexical's
default behaviour, exactly as today.

## Test strategy

Unit tests in `LexicalShell.test.tsx` (vitest + jsdom, existing
`renderComposer` / `pressEnter` helpers), one test per row of the table:

1. Enter in prose submits once, `defaultPrevented`, no newline inserted.
2. Shift+Enter in prose inserts a newline and never submits.
3. Mod+Enter still submits (Mac ⌘↵, Windows Ctrl+↵).
4. Enter on a disabled composer does nothing.
5. Enter inside a code block adds a line and does not submit; Mod+Enter
   inside a code block submits.
6. Enter inside a list item creates the next item and does not submit.
7. Enter on a ```` ``` ````-only line opens a code block and does not submit.

Branch check: `pnpm test`, `pnpm lint`, `pnpm typecheck` in `ade/web`.

## Work

Branch `feat/composer-enter-sends` off `feat/session-judge-provider`,
worktree `.worktrees/composer-enter-sends`. Tickets on the kanban board,
label `superpowers`, `composer-enter-sends`.
