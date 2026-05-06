# Workers TODOs

## audit-log: surface file-write errors instead of swallowing them

**What:** `audit-log/src/lib.rs::append_jsonl` currently logs+swallows I/O errors. The handler still replies `{"ok": true}` even when the audit line was never persisted.

**Why:** Audit gaps (full disk, read-only mount, permission error) go undetected. Defeats the point of audit logging.

**Repro:** test in `audit-log/tests/integration.rs` (`silent_on_unwritable_path`) shows the bug — handler returns success even when the path can't be written.

**Fix:** make `append_jsonl` return its `io::Result`; the handler propagates the error to the bus reply as `{"ok": false, "error": "..."}` so subscribers see the failure.

**Effort:** ~20 LOC in audit-log/src/lib.rs; update one test assertion to match new reply shape.

**Tracked here because:** introducing the new reply shape is a separate concern from the test-coverage PR that exposed it.
