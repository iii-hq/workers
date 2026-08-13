---
name: canvas
description: >-
  Create and edit diagrams as code — store mermaid text or excalidraw scenes
  under stable ids the console renders live, fetch the per-family mermaid
  syntax primer, and validate generated source before storing it.
---

# canvas

The canvas worker stores diagrams as editable source. A canvas is a named
record — mermaid text, or an excalidraw scene JSON for a freeform whiteboard —
kept under a stable 8-character id. The worker never renders anything; the
console does the drawing, so a `canvas::*` call appears in chat as the live
diagram and the canvas page lists and edits everything stored. The source is
the artifact, which is what keeps a diagram editable instead of frozen into
pixels.

Ids are stable across updates on purpose: revise a diagram as the
conversation evolves and every earlier reference to it keeps working. Records
persist in the `state` worker, so nothing is lost across a restart.

Mermaid should not be written from memory. `canvas::syntax` returns the
families the renderer actually supports with a working example each, and
`canvas::validate` parses source without storing it — together they mean a
broken diagram never lands in the store or renders as an error card.

## When to Use

- A diagram would say it better than prose — an architecture, a sequence of
  calls, a state machine, an ER model: create a mermaid canvas and let the
  console draw it.
- About to write mermaid source: call `canvas::syntax` first, narrowed to the
  family being written.
- Generated source in hand: `canvas::validate` before `canvas::create` or
  `canvas::update`.
- A diagram needs revising: `canvas::update` on the existing id, not a new
  canvas — the id is what earlier references point at.
- A spatial sketch a person will edit by hand on a whiteboard: create with
  format `freeform` and an excalidraw scene JSON as the source.
- Drawing while a person watches: create a freeform canvas, then
  `canvas::element::add` one shape or small group per call — the open
  console board shows each step as it lands.
- Finding what is already drawn: `canvas::list`, newest first, with an
  optional format filter; `canvas::element::list` for the shapes on one
  freeform board.

## Boundaries

- Nothing here renders. Headless callers get source back, never SVG or
  pixels; drawing happens in the console.
- Not a file store and not a document editor: the source is one diagram,
  capped by the configured `max_source_bytes`. Files belong to the shell and
  editor workers.
- `canvas::validate` checks that source parses, not that the diagram is any
  good — a valid diagram can still be the wrong diagram.
- A canvas's format is fixed at creation; `canvas::update` changes name and
  source, not format.
- `canvas::delete` is idempotent (`deleted=false` for an unknown id); the
  other id-taking functions error on unknown ids.

## Functions

- `canvas::create` — store a new canvas from mermaid text or an excalidraw
  scene JSON; mints the stable id and derives the mermaid family.
- `canvas::get` — read one canvas by id, editable source included.
- `canvas::list` — every stored canvas, newest first, optionally filtered by
  format; capped by the configured `max_list`.
- `canvas::update` — revise a canvas's name and/or source by id; the id never
  changes and the mermaid family is re-derived.
- `canvas::delete` — remove one canvas by id; unknown ids report
  `deleted=false` rather than erroring.
- `canvas::syntax` — the mermaid syntax reference: every supported diagram
  family with a summary and a working example, optionally one family.
- `canvas::validate` — parse source without storing it; reports validity, the
  derived family, and per-issue messages with line numbers where known.
- `canvas::element::add` — append shapes to a freeform canvas one drawing
  step at a time; the open console whiteboard shows each call as it lands.
- `canvas::element::update` — merge properties into one element of a
  freeform canvas by element id: move, recolor, or relabel a shape.
- `canvas::element::delete` — remove elements of a freeform canvas by
  element id; unknown ids are ignored and the response reports the count.
- `canvas::element::list` — the freeform board map: id, type, position,
  size, and text per element, read before updating or connecting shapes.

The `element` family works only on `format: freeform` canvases (excalidraw
scene source). Mermaid canvases are edited as text through `canvas::update`.
