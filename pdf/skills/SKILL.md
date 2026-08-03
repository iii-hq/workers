---
name: pdf
description: >-
  Read PDFs locally without OCR or an API key — classify text-based versus
  scanned in tens of milliseconds and name the pages that still need OCR,
  convert to markdown with headings, lists and tables intact, and pull
  positioned text or the exact characters inside a box on a page.
---

# pdf

The pdf worker parses PDF documents on the machine. A PDF is not text: reading
one with a file-reading function returns binary noise and spends the context on
it, so every PDF goes through `pdf::*` instead. Parsing is local, needs no
credential, and sends nothing anywhere.

Its first job is a routing decision. `pdf::classify` samples the document's
content streams and answers whether the pages hold real characters or are
photographs of pages, plus which individual pages cannot be read without a
vision model and why. That verdict decides whether the rest of the work is
worth doing at all, and it is what separates "this document is empty" from
"this document is a scan".

Its second job is reading. Text-based documents convert to markdown that keeps
the shape of the original, because headings, lists and tables are recovered
from font sizes and page geometry rather than from any structure the file
promises. Underneath that sit the positions themselves, for callers that need
to know where text is and not only what it says.

This worker is called on demand. It registers no harness hook and injects
nothing into any prompt, so a conversation that never touches a document never
pays for it. Reach for it when one appears.

## When to Use

- A conversation names a PDF path or hands one over: call `pdf::classify`
  before anything else. Never read a PDF with a file-reading function; it
  returns binary noise and spends the context on it.
- Read a document: `pdf::to-markdown`, narrowed with `pages` when it is long.
- Search or embed a document rather than read it: `pdf::extract-text`.
- Decide whether a document is worth sending to a vision model, and which of
  its pages: `pdf::classify`, then read `pages_needing_ocr` and `ocr_reasons`.
- A vision model located a region and you want the real characters rather than
  its transcription: `pdf::extract-regions`.
- Reason about layout, headings by size, or where a value sits on the page:
  `pdf::extract-items`.

## Boundaries

- Nothing here rasterizes a page, so nothing here can OCR. Scanned and
  image-based documents are classified and routed, never read. Image content
  is reported as a placeholder with a real box and no pixels.
- `suspected_garbled_text` in `ocr_reasons` means the text layer decodes to
  nonsense. Do not trust the extraction, whatever `document_type` says.
- Responses are capped. `truncated: true` with a much larger `total_chars`
  means you hold a fragment and must not answer from it. Narrow with `pages`
  rather than raising the cap: conversion cost scales with the document, so a
  page filter is faster as well as smaller. `max_chars: 0` lifts the cap and
  belongs in a pipeline moving a document to storage, not in a call whose
  result lands in the conversation.
- Encrypted documents take a `password` on `pdf::classify` and
  `pdf::to-markdown` only. The other three cannot decrypt and say so.
- Page numbers are 1-indexed everywhere, in requests and responses.
- Coordinates differ by function and every response states which it used:
  `pdf::extract-items` reports PDF points from the bottom left,
  `pdf::extract-regions` takes boxes in PDF points from the top left. Assuming
  the wrong one returns text from the wrong end of the page with no error.

## Functions

- `pdf::classify` — the routing call. Document type, confidence, page count,
  the 1-indexed pages needing OCR, and a machine-readable reason per page.
- `pdf::to-markdown` — markdown with headings, lists, links and tables
  recovered; optional page filter, per-page output, and a fidelity or compact
  profile.
- `pdf::extract-text` — plain text, no structure recovery. Cheaper than
  markdown when the result will be searched or embedded.
- `pdf::extract-items` — every positioned run of characters with its box,
  font, size, and recovered bold, italic, underline and strikeout.
- `pdf::extract-regions` — the text, or a markdown table, inside given boxes
  on given pages.
