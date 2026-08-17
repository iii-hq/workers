---
name: document
description: >-
  Read Word, PowerPoint, Excel, OpenDocument, RTF, EPUB and CSV files locally
  with no API key — detect the format from the bytes, convert to markdown with
  headings, lists and tables intact, and pull out the images markdown drops.
---

# document

The document worker converts office documents on the machine. A `.docx` or a
`.pptx` is a ZIP of XML: reading one with a file-reading function returns
compressed noise and spends the context on it, so every office document goes
through `document::*` instead. Conversion is local, needs no credential, and
sends nothing anywhere.

One serializer sits behind every format, so a `.doc` from 2003 and a `.pptx`
from yesterday come out with the same heading, table and list conventions. That
sameness is the point: a conversation handling a mixed bag of attachments reads
one shape, not fourteen.

The one thing markdown cannot carry is the pictures. An embedded image renders
as its alt text, which is right for prose and wrong for a deck of diagrams — a
deck whose content is images converts to a page of titles and reads as an empty
document. `document::to-markdown` reports how many images it dropped, and
`document::extract-assets` returns their bytes for a model that can see them.

This worker is called on demand. It registers no harness hook and injects
nothing into any prompt, so a conversation that never touches a document never
pays for it. Reach for it when one appears.

## When to Use

- A conversation names or hands over a `.docx`, `.doc`, `.pptx`, `.ppt`,
  `.xlsx`, `.xls`, `.odt`, `.ods`, `.odp`, `.rtf`, `.epub` or `.csv`: call
  `document::to-markdown`. Never read one with a file-reading function.
- A file whose type is unclear, or a batch to route: `document::detect` first.
  It reads the signature in the first bytes and answers in microseconds.
- The markdown came back thin and `asset_count` is above zero: the content is
  pictures. Call `document::extract-assets` and hand the images to a model that
  can see them.
- A PDF: prefer `pdf::classify` and `pdf::to-markdown` when the `pdf` worker is
  installed — it reports which pages are scans and need OCR. This worker
  converts text-based PDFs too, as a fallback.
- A document that came back with no text, or one classified as a scan: say so
  and offer `document::ocr` rather than running it unasked. It costs money per
  page. When you do run it, pass the `pages` that `pdf::classify` named.

## Boundaries

- `document::ocr` is the only function here that spends money, and the only one
  that needs other workers: `browser` to render a PDF's pages, and a vision
  model through llm-router. Both are optional installs; a call that needs one
  it cannot reach says which. Rendering a PDF needs `path`, not
  `bytes_base64`.
- Nothing here writes documents. Conversion is one-way, to markdown.
- Responses are capped. `truncated: true` with a much larger `total_chars`
  means you hold a fragment and must not answer from it. `max_chars: 0` lifts
  the cap and belongs in a pipeline moving a document to storage, not in a call
  whose result lands in the conversation.
- `document::extract-assets` is capped twice: how many assets come back, and
  how large one may be before its bytes are left out. Anything left out is
  still listed with its media type and size — an empty list means the document
  genuinely holds nothing.
- A CSV carries no signature, so it is recognised only by its file name. Inline
  bytes need `file_name` for it; every other format is read from the content.
- `detected_from: "extension"` on anything other than a CSV means the content
  matched nothing known and only the name suggested the format. Treat the
  result with more suspicion than a `content` detection.
- An encrypted document cannot be opened here at all. There is no password
  parameter; ask for an unlocked copy.

## Functions

- `document::detect` — what this file is, from its bytes: the format, the
  family (prose, spreadsheet, presentation, book, PDF), how it was recognised,
  and whether it can be converted. Microseconds, and no conversion.
- `document::to-markdown` — the document as markdown, with headings, lists,
  links, tables, footnotes and speaker notes preserved. Reports the count of
  embedded images it could not carry.
- `document::extract-assets` — the embedded images and objects as base64,
  filtered by media type, capped per response and per asset.
- `document::ocr` — transcribe a document that holds no readable text: a
  scanned PDF, a photographed page, a deck built out of pictures. Renders the
  pages and reads them with a vision model.
