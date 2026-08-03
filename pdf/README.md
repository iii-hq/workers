# pdf

Read PDFs on the machine, with no OCR service and no API key. This worker
classifies a document in about twenty milliseconds — is this real text, or a
photograph of a page? — converts text-based documents to markdown that keeps
their headings, lists, links and tables, and reports exactly which pages still
need OCR and why. It also exposes the layout underneath: where every run of
characters sits, and what the text is inside a given box on a page. Nothing is
uploaded, and a long document is capped rather than dumped, so a report does not
swallow the context an agent needed for the answer.

It ships a console page too. Drop a PDF in and see exactly what the agent sees:
the verdict, the per-page OCR decision, and the extracted markdown.

## Install

```bash
iii worker add pdf
```

## Quickstart

Classify first. It is cheap, and it decides whether anything else is worth
doing: extraction on a scan returns nothing, and without the verdict an empty
result is indistinguishable from an empty document.

```rust
use iii_sdk::{register_worker, InitOptions};
use iii_sdk::protocol::TriggerRequest;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let verdict = iii.trigger(TriggerRequest {
        function_id: "pdf::classify".into(),
        payload: json!({ "path": "/tmp/report.pdf" }),
        action: None,
        timeout_ms: Some(30_000),
    }).await?;
    // { "document_type": "text_based", "confidence": 1.0, "page_count": 40,
    //   "pages_needing_ocr": [], "ocr_reasons": [], "elapsed_ms": 18, … }

    let markdown = iii.trigger(TriggerRequest {
        function_id: "pdf::to-markdown".into(),
        payload: json!({ "path": "/tmp/report.pdf", "pages": [1, 2, 3] }),
        action: None,
        timeout_ms: Some(60_000),
    }).await?;
    // { "body": { "text": "# Quarterly Report\n…", "chars": 5693,
    //             "total_chars": 5693, "truncated": false }, … }

    println!("{markdown:#?}");
    Ok(())
}
```

A document with no path goes in as `bytes_base64` instead. An encrypted one
takes a `password` on `pdf::classify` and `pdf::to-markdown`.

### Reading the verdict

`document_type` is `text_based`, `scanned`, `image_based` or `mixed`. The
document-level answer is not the whole story: a two-hundred-page report with a
scanned cover is not a scanned document, and treating it as one sends the whole
thing to an OCR service for the sake of one page. `pages_needing_ocr` and
`ocr_reasons` carry the per-page decision:

| Reason | What it means |
|---|---|
| `scanned` | A raster page. It needs a vision model. |
| `no_text` | Nothing extractable and nothing to OCR. Often a blank page. |
| `vector_text` | Characters drawn as outlines rather than text. Unreadable as characters. |
| `suspected_garbled_text` | A text layer that decodes to nonsense. Do not trust it, whatever the document type says. |

### Response caps

Every text-bearing response is capped and says so. `truncated: true` with a
`total_chars` far above `chars` means you are holding a fragment.

The cheap fix is `pages`, not a bigger cap: conversion cost scales with the
document, so narrowing to the pages you need is faster as well as smaller. A
four-hundred-page report takes tens of seconds to convert whole and
milliseconds a page at a time.

`max_chars: 0` lifts the cap entirely. That belongs in a pipeline moving a
document to storage, not in a call whose result lands in a conversation.

### Reading a box on a page

When a vision model has located a region and you want the real characters
rather than its transcription:

```json
{
  "path": "/tmp/invoice.pdf",
  "regions": [{ "page": 1, "boxes": [[320.0, 640.0, 560.0, 700.0]] }],
  "mode": "text"
}
```

`mode: "table"` runs table detection over the same box and returns a markdown
table instead.

### Two conventions worth knowing

Page numbers are 1-indexed everywhere on this surface, in requests and
responses.

Coordinates are not uniform, and each response states which it used.
`pdf::extract-items` reports PDF points from the **bottom** left, the PDF
convention. `pdf::extract-regions` takes boxes in PDF points from the **top**
left, which is what a layout model produces. Getting this wrong is silent: text
comes back, just from the wrong end of the page.

## Configuration

Configuration lives in the `configuration` worker under the id `pdf` and every
field hot-reloads. Nothing here needs a restart.

```yaml
max_input_bytes: 268435456   # largest document accepted, before parsing
max_chars: 40000             # default cap on returned text or markdown
preview_chars: 600           # leading characters shown alongside a capped body
max_items: 5000              # default cap on positioned items in one response
classify_sample_pages: 8     # pages sampled to classify; 0 scans everything
min_text_ops_per_page: 3     # text operators before a page counts as text
text_page_ratio_threshold: 0.6  # share of text pages to call a document text-based
```

The three detection fields are the ones worth understanding. Sampling is what
keeps classification at tens of milliseconds on a four-hundred-page file; it
also means the verdict comes from part of the document, which is why every
response reports `pages_sampled`. Raise `classify_sample_pages`, or set it to
`0`, when a borderline mixed document needs settling.

Defaults live in [`src/config.rs`](src/config.rs).

## What this worker does not do

It does not rasterize pages, so it cannot OCR anything. Scanned and image-based
documents get classified and routed, not read. Image content is reported as a
placeholder with a real bounding box and no pixels.

It is a parser, not a renderer: it walks the document's content streams and
reconstructs the geometry, which is why it is fast and why it needs no service
behind it.
