---
name: slides
description: >-
  Author, draft, edit, present and export slide decks (HTML, PDF, PPTX) with
  a Console editor; use it whenever a person wants a presentation, a deck
  outline, or a document turned into slides.
---

# slides

The slides worker stores presentations as small structured documents: a
title, a theme, and ordered slides made of typed blocks (heading, text,
bullets, image, code, quote, metric). One document renders identically in the
Console editor, in a self-contained HTML presentation, in a PDF, and in a
PowerPoint file. Decks persist in the `state` worker and every change emits
the `slides::changed` trigger, so the Console page updates live while an agent
edits.

The worker owns storage, rendering and export. Drafting goes through the LLM
router and is the only function that needs a model; everything else works
without credentials.

## When to Use

- A person asks for a presentation, a pitch, a talk, a keynote outline, or a
  "deck" on any subject: draft it with `slides::outline` from their brief and
  source material, then refine slide by slide.
- Existing content (a report, notes, a Markdown document, meeting minutes)
  should become slides: convert it to the Markdown dialect and call
  `slides::create` in one step.
- A deck needs a delivery artifact: export PPTX for people who edit in
  PowerPoint or Keynote, PDF for sharing, HTML to present from a browser.
- A person wants to review or present a deck beside the chat: the Console
  page shows it; open it with `panels.open({ pageId: 'slides', context: { deck_id } })`.

## How to work

1. Draft first, then edit. `slides::outline` gives a complete, well-paced
   deck in one call; read it back with `slides::markdown` (compact) and fix
   individual slides with `slides::slide::update` rather than regenerating.
   Change one block (a diagram, a table, a caption) with
   `slides::block::update`; batch several edits into one `slides::apply`
   call with `expect_revision` so the person editing in the Console is not
   overwritten.
2. Inspect before you deliver. `slides::audit` reports, per slide and per
   block, what overflows, collides, clips or reads too small, plus missing
   notes and repeated words; fix from that JSON, then re-audit until
   `error_count` is zero. `slides::snapshot` shows you any slide as an
   image when a judgement call needs eyes.
3. Titles are takeaways, written as sentences. Bullets are short, at most
   five per slide. Prefer a metric, a quote or a statement slide over a
   fourth bullet slide.
4. Vary layouts: `title` once, `section` for chapters, `two-column` for
   comparisons and paired metrics, `statement` for the one line the room
   should remember, `image` only with a real URL.
5. Write speaker notes on every slide; they export to PowerPoint and show in
   the HTML presentation with `n`.
6. Pick a theme for the audience (`slides::themes::list`); set brand colors
   through `theme_overrides` instead of hard-coding them into text.

## Boundaries

- Charts and diagrams are data blocks (`chart`, `diagram`, `table`) drawn by
  the renderer; the worker does not generate raster images. Photos arrive as
  image URLs from another worker; the deck only places them.
- Snapshots, layout measurements and pixel-identical PDF and PPTX exports
  need the `browser` worker. Without it `slides::snapshot` fails with
  `CAPTURE_UNAVAILABLE`, `slides::audit` returns `measured: false` with the
  content checks only, and exports fall back to the native vector renderers
  (`engine: "native"` in the response).
- Not a document store for arbitrary files: exports go to `output_dir`, the
  deck itself lives in `state`.
- Drafting never invents facts beyond the brief; put figures and constraints
  in `context` so the model can use them.
- Rendering escapes everything and refuses unsafe image sources; the HTML
  presentation is self-contained but loads web fonts from the network.
- Write functions and exports are approval-gated for agents by default;
  reads, `render`, `audit` and `snapshot` are allowed.

## Triggers

- `slides::changed` fires after every create, update and delete with
  `{ kind, deck_id, revision, updated_at_ms }`. Bind with `{}` for all decks
  or `{ deck_id }` for one; the Console page uses it to refresh, never a
  timer.

## Failure and recovery

- `DECK_NOT_FOUND` / `SLIDE_NOT_FOUND` / `BLOCK_NOT_FOUND`: the id is stale;
  list decks or read the deck again before retrying.
- `REVISION_CONFLICT` from `slides::apply`: someone saved the deck since you
  read it. Read it again, rebuild the batch on the new revision, resend.
- `CAPTURE_UNAVAILABLE` / `CAPTURE_FAILED`: the browser worker is missing or
  its tab failed; install it, or continue with `slides::audit
  { measure: false }` and `slides::export { engine: "native" }`.
- `INVALID_THEME`, `INVALID_BLOCK`, `INVALID_SLIDE`: the payload broke the
  model; the message names the field. Fix the input, do not loop.
- `MODEL_UNAVAILABLE` or `DRAFT_INVALID_JSON` from `slides::outline`: the
  router has no chat model, or the model returned prose. Pass `model`, set
  `default_model` in the configuration, or retry once with a smaller
  `slide_count`.
- Export failures name the format; images that cannot be fetched render as a
  labelled placeholder rather than failing the export.
