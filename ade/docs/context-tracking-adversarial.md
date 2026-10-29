# Adversarial review: chat-owned Context tracking

Target: PR #1505, implementation commit `a4ef9fb0aa234b13783a9313ed70995274929d16`.
Console executable SHA-256: `ee1055111b48dbdede3cad406274ca9920cc31673ed426de88ba45f66b2d88a5`.

These tests run an actual isolated iii engine, configuration worker,
session-manager and the Console production bundle in Chromium. Each scenario has
fresh temporary storage and dynamically allocated loopback ports. No service or
SDK response is mocked. Source URLs are not fetched. The running operator stack
and its data are untouched. This review adds tests only, not runtime fixes.

## Execution results

Two complete real-stack runs produced the same result: **6 passed, 3 failed,
0 skipped, 0 flaky**, approximately 39.3 seconds each. The confirmation run at
`2026-10-22T00:05:56Z` captured Chromium's exact `Invalid time value` error and
the real list response accepting `updated_at: 9000000000000000`.

Local evidence: `target/context-adversarial-results.json` (first run),
`target/context-adversarial-confirmation/results.json` (confirmation), and the
corresponding `playwright-output/` directories with worker logs and traces.

## Findings

### ADV-01 — silent lost update (P1, data integrity)

Location: `ade/src/context_tracking.rs`, `save`.

1. Save a snapshot with progress 20 and retain the list result as writer A.
2. Writer B successfully saves progress 90 on the same id.
3. Writer A saves its old snapshot using the existing supported save contract.
4. List returns progress **20**, with a new timestamp, not 90 and not a conflict.

The shared lock prevents interleaved file writes but cannot detect an obsolete
read-modify-write. No revision/precondition exists. Two project conversations or
agents can silently discard each other's successful updates. The first run also
confirmed that an unknown `expected_updated_at` argument is silently ignored;
the confirmation run uses only supported fields to demonstrate the underlying
loss without relying on that extra argument.

Suggested remedy: server-issued monotonic revision and compare-and-swap for
edits/moves/removal, with an explicit conflict that prompts re-read and merge.

### ADV-02 — persisted timestamp crashes the chat surface (P2, recovery)

Location: `ade/web/src/components/chat/ContextTracking.tsx`, `toISOString`.

1. Save a legitimate snapshot.
2. Fault-inject `updated_at: 9000000000000000` into the temporary JSON file.
3. This is valid JSON and a valid Rust u64; list accepts it.
4. Open the conversation in real Chromium: the Tracking region disappears as
   rendering fails on `new Date(updated_at).toISOString()` (outside Date range).

This value cannot currently be created through public save, which stamps time
server-side. It requires malformed/migrated/hand-edited persistence; it is a
recovery robustness bug, not a demonstrated remote timestamp injection. Syntactic
JSON corruption is handled, but semantically invalid persisted values are not.

Suggested remedy: validate persisted records at load, isolate invalid items, and
format timestamps defensively so one record never crashes the chat surface.

### ADV-03 — open project viewer stays stale after another conversation saves (P2, UX)

Location: `ade/web/src/components/chat/ContextTracking.tsx`, read invalidation.

1. Two real sessions use the same project directory.
2. Viewer A has the project snapshot open at progress 20.
3. Writer B successfully saves progress 85 on the same project item.
4. The persisted value is 85, but A's open progressbar remains **20** for the
   observation window; there is no tracking-change subscription.

The documented MVP reloads at focus/panel/turn/reconnect boundaries, so this is a
project-sharing usability gap rather than a violation of a promise to monitor
the external release source live. Source monitoring and propagation of already
saved snapshots are distinct. An idle project viewer has no indication of the
newer saved snapshot until another invalidation occurs.

Suggested remedy: emit scoped tracking-changed notifications on successful
save/remove and invalidate matching visible conversation/project snapshots.
No source polling or background agent is needed.

## Defenses that passed

- 24 malformed snapshot variants were rejected without changing stored bytes:
  blank/oversized/UTF-8 titles, invalid summary/progress types and bounds,
  duplicate/oversized checklist entries, unsafe or credential-bearing source
  URLs, and an unknown nested executable field. Valid boundary/fractional
  progress values remained accepted.
- Another conversation's private item and another project's items were not
  editable/removable via guessed ids and forged owner/project arguments.
  A nonexistent session was rejected. This does **not** establish multi-user
  authorization: direct SDK clients can select session ids by design, within
  the existing trusted engine boundary.
- Canonical symlink aliases shared project snapshots; subdirectories and an
  explicitly changed project did not inherit the old project's items.
- 32 concurrent creates retained all writes. A further 78 simultaneous creates
  accepted exactly 68, enforcing 100 total. Moving into that full scope failed
  without changing the file; editing an existing item still worked.
- Truncated storage was preserved rather than overwritten. A browser with a
  cached snapshot showed the error, retained its prior progress and recovered
  through Retry after the actual file was restored.
- Stored HTML/script/image event-handler payloads stayed text in Chromium;
  no injected elements or side effects were observed and no source URL was
  requested. The malicious prose did not enter system guidance. This is **not**
  an LLM prompt-injection resistance evaluation of later tool-result content.

## Reproduce

```sh
cd ade/web
CONTEXT_ADVERSARIAL=1 \
CONTEXT_ENGINE_BIN=/absolute/path/to/iii \
CONTEXT_SESSION_BIN=/absolute/path/to/session-manager \
CONTEXT_CONSOLE_BIN=/absolute/path/to/worktree/ade/target/debug/ade \
PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH=/absolute/path/to/chrome \
CONSOLE_E2E_ARTIFACTS_DIR=/absolute/path/to/artifacts \
PLAYWRIGHT_JSON_OUTPUT_FILE=/absolute/path/to/artifacts/results.json \
pnpm exec playwright test e2e/context-tracking-adversarial.spec.ts --reporter=line,json
```

Tests intentionally assert the safe/desired outcome, so the three findings remain
red against the reviewed implementation. They require the explicit
`CONTEXT_ADVERSARIAL=1` opt-in, even when the stack binaries are configured;
ordinary E2E runs leave this diagnostic suite out. Do not mark them expected failures to
claim the implementation passes. The fixture captures real worker logs; failing
browser scenarios retain screenshots, videos and Playwright traces. JSON test
attachments contain stale/new/actual values and the external project's saved
record. Artifacts are local and may contain temporary paths and generated session
ids, so only this sanitized report is intended for the PR.
