# chat-app

A base scaffold for a chat surface, built with Vite + React + TypeScript +
Tailwind v4 and styled to the iii Schematic design system
(see [`../DESIGN.md`](../DESIGN.md) for the full spec).

It runs entirely client-side with mocked streaming, so there are no API keys
to configure. The mock — and an interactive Playground that exercises every
streaming edge case (errors, aborts, multi-tool runs, long markdown, …) —
ships behind the `VITE_PLAYGROUND` flag, on by default in dev and off in
prod. Drop a real provider in by replacing one file
(see [Swapping in a real backend](#swapping-in-a-real-backend) below) and
[`PLAYGROUND.md`](./PLAYGROUND.md) is the contract you have to honor.

## Quickstart

```bash
npm install
npm run dev
```

Then open the printed `Local:` URL (Vite picks the first free port from
5173 upwards).

## Scripts

| command            | what it does                              |
| ------------------ | ----------------------------------------- |
| `npm run dev`      | Start the Vite dev server with HMR.       |
| `npm run build`    | Type-check, then build a static bundle.   |
| `npm run preview`  | Serve the built bundle locally.           |
| `npm run typecheck`| Type-check without emitting.              |

## What's in the box

- **Composer** powered by [`lexical`](https://lexical.dev/) in plain-text
  mode. The plugin layer is intentionally thin so that autocomplete, mention
  pickers, or slash menus can be added later without restructuring the
  editor.
- **Markdown** rendering via `react-markdown` + `remark-gfm`, with element
  renderers that follow the iii Schematic (lowercase headings, monospace
  body, bordered code blocks and tables).
- **Backend seam** in [`src/lib/backend/`](src/lib/backend/). `ChatView`
  consumes a `ChatBackend` interface that yields a documented stream of
  events; the mock (dev) and the stub real backend (prod) both implement it.
  Three canned bodies — one per mode — exercise headings, lists, fenced
  code, blockquotes, and inline code on the first run.
  See [`PLAYGROUND.md`](./PLAYGROUND.md) for the full contract.
- **Playground** at `#/playground` (dev only) — a chat surface driven by a
  catalog of scenarios (errors, aborts, multi-tool runs, slow/fast streams,
  long markdown) that stress every corner of the streaming contract. Useful
  before swapping in a real backend.
- **Model picker** and **mode picker** (`plan` / `ask` / `agent`) wired into
  the canned response so you can see the values flow through.
- **File attachments** via a hidden file input. Previewable text/image
  files store a data URL; binaries store metadata only. Attachments are
  cleared after the next outgoing message.
- **Sidebar** listing conversations, persisted to `localStorage` under
  `iii-chat-conversations`. Double-click a row to rename inline; hover to
  reveal the delete affordance.
- **Light / dark theme** toggle, persisted under `iii-theme` and applied
  pre-paint to avoid a flash.

## Layout

```
src/
  main.tsx
  App.tsx                # routing + flag-guarded lazy() for dev pages
  index.css              # Tailwind v4 + iii Schematic tokens + utilities
  lib/
    utils.ts             # cn = twMerge(clsx(...))
    storage.ts           # localStorage CRUD
    markdown.tsx         # iii-styled react-markdown wrapper
    backend/             # ← the seam. ChatBackend interface + impls
      types.ts           #     StreamEvent, ChatBackend, ChatStreamOptions
      mock.ts            #     dev-only mock; tree-shaken in prod
      real.ts            #     ← swap this stub for your provider
      index.ts           #     getDefaultBackend() picks one based on flag
  types/chat.ts          # Conversation, Message, Mode, ModelId, Attachment
  hooks/
    use-conversations.ts # state + persistence
    use-hash-route.ts    # #/ #/playground #/examples
    use-theme.ts         # theme + persistence
  components/
    ui/                  # iii Schematic primitives
    sidebar/             # ConversationSidebar + ConversationRow
    chat/                # ChatView, Composer, LexicalShell, Message, etc.
  pages/
    Chat.tsx             # the production chat surface
    Examples/            # spec sheet of component variants (dev only)
    Playground/          # interactive scenario sandbox (dev only)
      scenarios/         # one ChatBackend per file
```

## Swapping in a real backend

Open [`src/lib/backend/real.ts`](src/lib/backend/real.ts) and replace the
stub generator with one that talks to your provider. The shape of the
events you yield is defined in
[`src/lib/backend/types.ts`](src/lib/backend/types.ts) and explained in
[`PLAYGROUND.md`](./PLAYGROUND.md). As long as your generator yields
`StreamEvent`s in the documented order, the chat surface and every
playground scenario keep working — that's the whole point of the seam.

A sketch for OpenAI's chat-completions stream:

```ts
import type { ChatBackend } from './types'

export const realBackend: ChatBackend = {
  id: 'openai',
  async *stream(prompt, _mode, model, opts) {
    const res = await fetch('https://api.openai.com/v1/chat/completions', {
      method: 'POST',
      signal: opts?.signal,
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${import.meta.env.VITE_OPENAI_API_KEY}`,
      },
      body: JSON.stringify({
        model,
        stream: true,
        messages: [{ role: 'user', content: prompt }],
      }),
    })
    // ... read res.body as a ReadableStream, parse SSE chunks,
    //     yield { kind: 'assistant-token', token } as each delta arrives,
    //     finish with { kind: 'assistant-end' }.
  },
}
```

To verify your implementation against the same edge cases the mock survives,
flip the flag on (`VITE_PLAYGROUND=1 npm run dev`), open `#/playground`, and
walk every scenario in the picker. If they all render correctly, your
backend is contract-clean.

## Design system

Every primitive in [`src/components/ui`](src/components/ui) is ported
verbatim from §10 of [`../DESIGN.md`](../DESIGN.md). The theme tokens in
[`src/index.css`](src/index.css) are from §0 of the same document. If you
change anything visual, mirror the change in `DESIGN.md` — the doc is the
source of truth, not the code.
