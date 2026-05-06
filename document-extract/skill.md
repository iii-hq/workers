# document-extract

Extract UTF-8 text from PDF or DOCX files on disk for agent context ingestion.

`document::extract({ path, max_bytes? }) → { text, page_count?, metadata: { byte_size, sniffed_mime }, detected_format }`

Reads the file at `path`, sniffs its format from magic bytes (PDF `%PDF-` prefix or DOCX ZIP signature), and returns the extracted text. `detected_format` is `"pdf"`, `"docx"`, or `"auto"` (unrecognised format). `page_count` is populated only for PDFs (best-effort marker scan); DOCX always omits it.

## When to use

- Agent needs to read a user-uploaded PDF or DOCX into context before answering questions about it.
- Ingesting contract, report, or form content for downstream processing or summarisation.
- Checking whether a document is parseable before storing or forwarding it.

## Notes

- Default `max_bytes` is 25 MiB (26,214,400 bytes). Override globally via the `DOCUMENT_EXTRACT_MAX_BYTES` env var or per-call via the `max_bytes` payload field; per-call value takes precedence.
- Format is detected from magic bytes, not the file extension; a `.pdf` file with a DOCX ZIP header is treated as DOCX.
- `path` must be an absolute filesystem path accessible to the worker process.
- Files larger than `max_bytes` are rejected before reading; the handler returns an error rather than a partial result.
- `metadata` always carries `byte_size` (u64) and `sniffed_mime` (string); no other keys are guaranteed.
