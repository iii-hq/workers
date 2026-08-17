# document

Read office documents on the machine, with no conversion service and no API
key. This worker takes a Word, PowerPoint, Excel, OpenDocument, RTF, EPUB or CSV
file and returns markdown that keeps its headings, lists, tables and notes, in
single-digit milliseconds for a typical document. It identifies a file from its
bytes rather than trusting its name, so a mislabelled attachment still converts.
And it hands back the images markdown cannot carry, which is what a deck of
diagrams actually holds. Nothing is uploaded, and a long document is capped
rather than dumped, so a report does not swallow the context an agent needed for
the answer.

## Install

```bash
iii worker add document
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions};
use iii_sdk::protocol::TriggerRequest;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let markdown = iii.trigger(TriggerRequest {
        function_id: "document::to-markdown".into(),
        payload: json!({ "path": "/tmp/quarterly.docx" }),
        action: None,
        timeout_ms: Some(60_000),
    }).await?;
    // { "format": "docx", "family": "prose", "detected_from": "content",
    //   "body": { "text": "# Quarterly Notes\n…", "chars": 5693,
    //             "total_chars": 5693, "truncated": false },
    //   "asset_count": 0, "elapsed_ms": 4, … }

    println!("{markdown:#?}");
    Ok(())
}
```

A document with no path goes in as `bytes_base64` instead — the shape a composer
attachment takes. Add `file_name` with it: a CSV carries no signature of its
own, and without a name it cannot be recognised.

## Formats

| Format | Extensions |
|---|---|
| Word | `.doc`, `.docx`, `.docm` |
| PowerPoint | `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm` |
| Excel | `.xls`, `.xlsx`, `.xlsm`, `.xlsb` |
| OpenDocument | `.odt`, `.ods`, `.odp` |
| Rich Text | `.rtf` |
| EPUB | `.epub` |
| CSV | `.csv` |
| PDF | `.pdf` (text-based; see below) |

Container variants collapse onto one name: `.docm` is `docx`, `.xlsb` is
`excel`. A caller matches on the format, never on the extension it happened to
send.

### PDFs

A text-based PDF converts here, which makes this worker a complete answer for a
mixed pile of attachments on its own. When the [`pdf`](../pdf) worker is
installed it is the better route for them: it classifies scanned versus
text-based and names the individual pages that need OCR, where this worker can
only convert or fail.

## Detect before you convert

`document::detect` reads the signature in the first bytes of a file and answers
in microseconds. It exists for the case where something arrives and nobody knows
what it is.

```json
{
  "format": "pptx",
  "family": "presentation",
  "detected_from": "content",
  "convertible": true,
  "has_assets": true,
  "size_bytes": 184320,
  "source": "roadmap.pptx",
  "elapsed_ms": 0
}
```

`detected_from` is the field worth reading. `content` means the bytes named the
format, which is the strong answer. `extension` means they did not, and only the
file name suggested it — expected for a CSV, and a reason for suspicion on
anything else. A `format` of `null` is an answer too: this is not a document
this worker reads, not a document that is broken.

## The images markdown drops

Markdown renders an embedded image as its alt text. For prose that is right. For
a deck built out of diagrams it throws away the content and leaves a page of
titles, which reads as a document that had little to say.

`document::to-markdown` reports `asset_count` so that case is visible, and
`document::extract-assets` returns the bytes:

```json
{
  "format": "pptx",
  "assets": [
    {
      "index": 0,
      "media_type": "image/png",
      "origin_part": "ppt/media/image1.png",
      "size_bytes": 48211,
      "bytes_base64": "iVBORw0KGgo…"
    }
  ],
  "total_count": 1,
  "truncated": false
}
```

Three ceilings apply, and all of them report what they dropped rather than
trimming silently. `max_assets` bounds how many come back. `max_asset_bytes`
bounds one payload, and `max_assets_total_bytes` bounds the response as a whole,
because two dozen assets each just under the per-asset limit still add up to a
quarter of a gigabyte once base64 inflates them. An asset left out either way is
still listed with its type and size, with `omitted` saying which ceiling it hit
(`too_large` or `budget_spent`), so a caller can ask for it on its own.
`include_bytes: false` inventories a document without moving anything.

## Reading a scan

A scanned page holds no text to extract: the characters exist only in the
pixels. `document::ocr` renders those pages and reads them with a vision model.

```json
{
  "via": "pdf-render",
  "body": { "text": "INVOICE 4471\nDue 30 June…", "chars": 812, "truncated": false },
  "pages": [{ "page": 1, "text": "INVOICE 4471…", "chars": 812, "cached": false }],
  "pages_transcribed": 1,
  "pages_cached": 0,
  "model": "claude-haiku-4-5"
}
```

Three inputs, one answer. An image goes straight to the model. A PDF is
rendered a page at a time by the [`browser`](../browser) worker, which is the
only thing that turns a page into pixels. An office document whose text came
back empty has its embedded images pulled out and read the same way.

Both of those dependencies are soft. Neither is declared in
`iii.worker.yaml`, every other function works without them, and a call that
needs one it cannot reach says which to install. Someone who installed this
worker to read a `.docx` never pays for Chromium.

This is the one function here that costs money, so nothing runs it implicitly.
`pdf::classify` reports which pages are scans, and passing that list is the
difference between transcribing one page of a report and all four hundred:

```json
{ "path": "/tmp/report.pdf", "pages": [1], "model": "claude-haiku-4-5" }
```

The model is checked for vision support before anything is rendered, because a
model that cannot see fails on the first page after the render has been paid
for.

Page transcriptions cache in the `state` worker, keyed by the rendered PIXELS
and the model that read them. Keying on the image rather than the source
document is what makes the cache self-correcting: a page that rendered badly
hashes differently once the render is fixed, so bad entries fall out instead of
being served forever. A hit still re-renders — that is a second of local
Chromium — and skips the model call, which is the part that costs money. The
images themselves are never stored, and exist only in flight between the browser
and the model.

For scale: one rendered page of a text PDF measured about 1,400 input tokens on
`claude-haiku-4-5`, or roughly $0.0016 a page.

Rendering a PDF needs the file on disk (`path`, not `bytes_base64`) and the
`browser` worker allowed to open it: its Behavior settings carry an allowed
URL schemes list that ships as `http, https`, and a local PDF needs `file`
added. It hot-applies on save. That list is deliberately narrow — the browser
does not check a path against the session's filesystem scope the way this
worker does, so widening it widens what any caller can read.

## Response caps

Every text-bearing response is capped and says so. `truncated: true` with a
`total_chars` far above `chars` means you are holding a fragment. `max_chars: 0`
takes the whole document, and belongs in a pipeline moving a document to
storage rather than in a call whose result lands in a conversation.

## Configuration

Configuration lives in the `configuration` worker under the id `document` and
every field hot-reloads. Nothing here needs a restart.

```yaml
max_input_bytes: 67108864   # largest document accepted, before parsing
max_chars: 40000            # default cap on returned markdown
preview_chars: 600          # leading characters shown alongside a capped body
max_assets: 24              # assets returned in one response
max_asset_bytes: 8388608    # largest single asset returned with its bytes
max_assets_total_bytes: 33554432  # total asset payload one response may carry
ocr_model:                  # vision model document::ocr reads with; unset = every call chooses
max_ocr_pages: 20           # pages one document::ocr call transcribes
ocr_timeout_ms: 120000      # budget for one render or one model read
ocr_render_settle_ms: 2000  # let a rendered page paint before capturing it
ocr_cache: true             # cache page transcriptions in the state worker
```

A per-call `max_assets` narrows this ceiling and cannot raise it: the limit
bounds one response, and a caller asking for a thousand images is the case it
exists for.

Defaults live in [`src/config.rs`](src/config.rs).

## Called on demand

This worker registers no harness hook and injects nothing into any prompt. A
conversation that never touches a document never pays for it, and there is no
per-turn cost to having it installed. An agent finds it the ordinary way,
through the function registry and [`skills/SKILL.md`](skills/SKILL.md).

## What this worker does not do

It does not run OCR by itself. `document::ocr` renders and asks a model, which
means a scan costs money per page and needs a vision model configured. Nothing
transcribes implicitly.

It does not write documents. Conversion is one way, into markdown.

It cannot open an encrypted document. There is no password parameter, because
there is nothing behind it that could decrypt one.
