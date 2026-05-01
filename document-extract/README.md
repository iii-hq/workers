# document-extract

PDF and Word text extraction on the iii bus. Used by `document::extract` to
ingest user-uploaded files for agent context.

## Installation

```bash
iii worker add document-extract
```

## Run

```bash
iii-document-extract --engine-url ws://127.0.0.1:49134
```

## Registered functions

| Function | Description |
|---|---|
| `document::extract` | `{ path \| bytes, format? }` → `{ text, page_count?, metadata: { byte_size, sniffed_mime } }`. Output text is always UTF-8. |

Page count is best-effort and only populated for PDFs.

## Build

```bash
cargo build --release
```
