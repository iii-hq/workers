# slides

Slide decks as an iii worker. `slides::*` functions author, store, draft, render and export presentations, and a **Slides** page in the Console edits and presents them. A deck is a small structured document (title, theme, ordered slides, typed blocks: heading, text, bullets, image, code, quote, metric) that renders the same way in the editor, in the self-contained HTML presentation, in the PDF, and in the PowerPoint file. Your agent writes the deck in one call, from JSON or from Markdown; `slides::outline` drafts a whole deck from a brief through the LLM router.

![The Slides editor in the iii Console](https://raw.githubusercontent.com/iii-hq/workers/main/slides/assets/slides-console.png)

## Install

```bash
iii trigger compose::add worker=slides@latest
```

`iii trigger compose::add` declares the worker in `worker-compose.yaml` and starts it as part of the Compose project. It needs the `state` worker for storage and the `ade` Console for the page. `slides::snapshot`, `slides::audit` measurements and the pixel-identical PDF and PPTX exports compose the `browser` worker over the bus (`compose::add worker=browser@latest`); without it exports fall back to the native vector renderers and audits keep their content checks. `llm-router` is only used by `slides::outline`, so every other function works without any API key.

## Quickstart

Open **Slides** from the Console navigation, or press `⌘K` and run `Open Slides`. **New deck** starts an empty deck; **Draft** turns a brief into a complete deck with takeaway titles, varied layouts, metric slides and speaker notes. Click any text on the slide to edit it in place, pick layouts and themes in the inspector, reorder slides in the rail, then **Present** or **Export** as PowerPoint, PDF or HTML.

The same deck is one function call away. Markdown is the fastest way in:

```bash
iii trigger slides::create markdown='# Launch plan
What ships in October
---
## Three bets
- Faster onboarding
- Usage-based pricing
- Partner API
<!-- notes: keep this to one minute -->
---
<!-- layout: statement -->
> Make it work, make it right, make it fast.
> — Kent Beck'
```

```json
{
  "deck": {
    "id": "deck-3f9c1a2b",
    "title": "Launch plan",
    "subtitle": "What ships in October",
    "theme": "midnight",
    "slides": [
      { "id": "slide-8a1e", "layout": "title", "title": "Launch plan", "subtitle": "What ships in October", "blocks": [] },
      { "id": "slide-c04d", "layout": "content", "title": "Three bets", "blocks": [{ "id": "block-1", "type": "bullets", "items": ["Faster onboarding", "Usage-based pricing", "Partner API"] }], "notes": "keep this to one minute" },
      { "id": "slide-77b2", "layout": "statement", "blocks": [{ "id": "block-2", "type": "quote", "text": "Make it work, make it right, make it fast.", "attribution": "Kent Beck" }] }
    ],
    "revision": 1
  }
}
```

Slides are separated by a line containing `---`. `#` on the first slide is the deck title, `##` is a slide title, `-` lines are bullets, `>` is a quote (a trailing `— name` line is the attribution), `![alt](url)` is an image, fenced code is a code block, and `<!-- layout: section -->`, `<!-- notes: ... -->`, `<!-- background: #101010 -->` are directives. Structured input uses `slides: [{ layout, title, subtitle, blocks, notes }]` instead.

Then:

```bash
iii trigger slides::outline topic='Q3 results for the board' audience='Board members' slide_count=10
iii trigger slides::export deck_id=deck-3f9c1a2b format=pptx
iii trigger slides::render deck_id=deck-3f9c1a2b
```

`slides::outline` returns the persisted deck and the model that drafted it; `slides::export` writes the file under `output_dir` (`~/.iii/slides` by default) and returns its path, or the bytes as `data_base64` with `inline: true`; `slides::render` returns the HTML presentation (arrow keys, `n` for speaker notes, `f` for fullscreen, print to PDF from the browser). `slides::get`, `slides::markdown`, `slides::update`, `slides::slide::insert|update|remove|reorder`, `slides::block::insert|update|remove|reorder`, `slides::apply`, `slides::delete`, `slides::list` and `slides::themes::list` complete the editing surface; each function's description carries its request shape.

An agent gets eyes and hands on a deck without a screen. `slides::snapshot { deck_id, slide? }` renders any slide (or an `overview` contact sheet) headlessly and returns it as an image, exactly as the presentation and the exports look. `slides::audit { deck_id }` returns machine-readable diagnostics per slide: overflow after auto-fit, smallest body font size, block collisions, clipped labels, empty-space ratio, word and bullet counts, missing notes, repeated words, ragged tables, each finding tagged with the block id to fix. `slides::block::update` changes one diagram, table, chart or caption without resending the slide, and `slides::apply { deck_id, ops, expect_revision }` runs a batch of slide and block operations in one revision increment, rejecting the batch with `REVISION_CONFLICT` when the deck changed underneath it so the agent and the Console editor never overwrite each other.

```bash
iii trigger slides::audit deck_id=deck-3f9c1a2b
iii trigger slides::snapshot deck_id=deck-3f9c1a2b slide=2 scale=0.5
iii trigger slides::apply deck_id=deck-3f9c1a2b expect_revision=4 ops='[{"op":"block.update","slide_id":"slide-c04d","block_id":"block-1","block":{"items":["Faster onboarding","Partner API"]}}]'
```

The worker emits the `slides::changed` trigger type (`{ kind, deck_id, revision, updated_at_ms }`) whenever a deck is created, updated or deleted; bind it with an empty config, or `{ deck_id }` for one deck. Another worker opens a deck in the page with `host.panels.open({ pageId: 'slides', context: { deck_id } })`.

### Themes

`midnight`, `paper`, `aurora`, `slate`, `sunrise` and `forest` ship built in; `slides::themes::list` returns their palettes and fonts. A deck overrides its theme with `theme_overrides: { accent, background, ink, font_heading, font_body, footer }`, and the configuration's `brand_accent` and `brand_footer` apply to every deck unless the deck overrides them.

### Deck designer agent

The worker ships an agent profile, [`agents/deck-designer.md`](https://github.com/iii-hq/workers/blob/main/slides/agents/deck-designer.md): an identity that extends the bundled `iii` base with presentation-design doctrine and preloads the `slides::*` functions. `directory::skills::download { worker: "slides" }` routes it into the directory's `agents_folder` (copying the file there by hand works the same); then `harness::send { options: { agent: "deck-designer" } }`, or the Console's agent picker, runs it.

## Configuration

The worker registers the id `slides` with the `configuration` worker on first boot; after that the live value is authoritative and hot-reloads, so the Console's global Settings modal is the place to change it.

```yaml
output_dir: ~/.iii/slides          # where exports are written
default_theme: midnight            # theme for decks created without one
default_model: ""                  # router model for slides::outline; empty = first chat model in the catalog
default_provider: ""               # router provider paired with default_model
outline_max_output_tokens: 8000    # cap on one drafted deck
brand_accent: ""                   # hex color applied to every deck
brand_footer: ""                   # footer text on every slide
```

An optional `--config path.yaml` seeds these values on first boot. `engine_url` (or `--url` / `III_URL`) is bootstrap and never hot-reloads.

## Security

Decks live in the `state` worker under the `slides_decks` scope. Exports write only under `output_dir` unless a call passes an absolute `path`. Rendered HTML escapes every string and drops image sources that are not `http(s)`, root-relative or `data:image/`. Read-only functions (`list`, `get`, `markdown`, `render`, `themes::list`, `audit`, `snapshot`) are allowed for agents by default; everything that writes a deck, exports a file or spends router tokens needs approval. Headless captures write the rendered HTML to a private temporary directory that is removed after the browser session closes.
