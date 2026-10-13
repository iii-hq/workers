# WhatsApp worker performance review

Review date: October 5, 2026. The main opportunities are archive query work and
large-payload memory use. This review adds reproducible benchmarks and evidence;
it does not change worker behavior or deploy a new binary.

## Scope and measurement conditions

The reviewed source is `workers/.worktrees/whatsapp-go/whatsapp`, branch
`feat/whatsapp-go`, at base commit
`d6788be0fb99cbc221ea3b42c60e3131e0785ab2` **plus the existing uncommitted Cloud API
and UI changes**. The base commit alone does not reproduce these results.

Measurements used Go 1.27.1, linux/amd64, an Intel Core i9-14900K, one Go scheduler
CPU (`-cpu=1`), six samples per workload, and a 300 ms minimum measurement interval.
Benchmarks ran serially on this shared development host. The pinned
`modernc.org/sqlite v1.60.1` reports SQLite 3.53.4. Fixtures use temporary on-disk
databases, the production schema, `journal_mode=DELETE`, `secure_delete=ON`, and
one SQL connection. Seeding with a prepared transaction is outside timing.

Archive fixtures contain short synthetic text, no expired messages, and either
100 conversations or one large conversation. Cloud replay fixtures alternate
incoming/outgoing rows and have no stored receipts or event markers. These are
controlled workloads, not live-session latency or throughput measurements.
Meta requests, real messages, production archives, credentials, session state,
and tunnel configuration were not used.

[Raw samples](performance/2026-10-05/baseline.txt),
[query plans](performance/2026-10-05/query-plans.txt), and
[reviewed source hashes](performance/2026-10-05/source-sha256.txt) identify the
measurement inputs. Allocated bytes per operation are cumulative allocation,
not peak resident memory. Times below are medians; full ranges are in the raw
samples. There is no before/after implementation comparison or claimed speedup.

## Confirmed findings and implementation order

### 1. Native receipts scan the conversation instead of seeking the message

[`recordReceipt`](../inbox.go#L464) filters by `chat_id`, `message_id`, and
`from_me`, omitting the middle key of the existing unique index
`(chat_id,sender_key,message_id)`. The actual plan seeks only `chat_id` through
`inbox_chat_order`. A single repeated receipt costs 0.179 ms at 1,000 messages,
2.053 ms at 10,000, and 25.651 ms at 100,000 in the same conversation. The focused
CPU profile places the dominant work in SQLite's VM and B-tree traversal.

The smallest candidate is adding `sender_key='me'` while retaining `from_me=1`
and the existing monotonic status expression. The candidate plan seeks all three
unique-index keys. Supported historical Go writers already use `me` for outgoing
rows; automatic import of the former Rust archive is unsupported. Verify receipt
collision isolation, replay, group handling, and status ordering before changing
the SQL, then compare this benchmark on the same toolchain.

### 2. Deep message pagination scans newer rows

[`ListMessages`](../inbox.go#L647) uses an OR cursor predicate with independently
bound timestamp parameters. Its plan seeks only `chat_id`. Reading 100 messages
below the newest 90% costs 0.869 ms at 10,000 retained messages and 7.625 ms at
100,000. The latest page stays approximately 0.25 ms at both sizes.

The candidate tuple predicate `(timestamp_ms,row_id)<(?,?)` enables a timestamp
range seek in this pinned SQLite. Preserve descending ordering and same-timestamp
tie handling. Planner checks must use bound parameters: substituting timestamp
literals allows constant folding and misleadingly produces a better plan for the
current expression. Measure the actual method after the change and test page
boundaries, equal timestamps, and cursor isolation.

### 3. Incoming Cloud retries pay archive-wide reconciliation and retention costs

[`recordExternal`](../inbox.go#L535) runs two outgoing-message reconciliation
UPDATEs even for incoming-only deliveries without receipts. Their outer queries
scan `inbox_messages`; receipt lookups inside the scan are indexed.
[`trimInbox`](../inbox.go#L307) also walks retained messages and markers with
OFFSET pruning on every write. Receipt pruning sorts without a matching ordering
index when receipts exist.

A fixed one-message incoming replay, without archive growth or new event markers,
costs 0.304 ms at 1,000 rows, 1.924 ms at 10,000, and 21.948 ms at 100,000. This
measures the combined method, not an isolated attribution to any one statement.
Populated receipt/marker tables and new deliveries need separate benchmarks.

First narrow reconciliation to affected receipt IDs and newly stored outgoing
IDs, preserving receipts that arrive before their message and authoritative sent
timestamps. Then measure pruning independently before changing its accounting or
adding an ordering index. Immediate retention, secure deletion, dedup markers,
clear/account changes, and transaction atomicity must remain intact.

### 4. Truncated Cloud previews retain large backing strings

[`clipInboxText`](../inbox.go#L214) returns a substring.
[`cloudMessagePreview`](../cloud_webhook.go#L487) clips it to 16 KiB and
[`boundedCloudEvent`](../cloud_delivery.go#L12) can clip again to 8 KiB while
omitting oversized raw data. Keeping the small event still keeps the full decoded
text allocation alive.

An isolated experiment parsed 16 synthetic webhook messages with 1 MiB text each,
retained only their bounded events, discarded parser inputs and batches, and ran
GC twice. The heap delta was 16,801,192 bytes for 131,072 bytes of visible preview
text. Cloning only the retained preview strings in the experiment reduced the
delta to 163,272 bytes. An `inuse_space` profile identified the retained decoded
JSON strings. These atypically large parser-accepted messages demonstrate the
mechanism; their frequency in real Meta deliveries was not measured.

Candidate: clone strings when truncation occurs, then measure ordinary messages
as well as oversized fields. The experiment does not change the worker. Its
[source](performance/2026-10-05/preview-retention_test.go.txt) and
[output](performance/2026-10-05/preview-retention.txt) are retained for reproduction.

### 5. Upload request decoding allocates several copies of the encoded file

[`decodeCloudRequest`](../cloud.go#L526) serializes an already decoded map and
decodes the JSON again into a typed request. Document and media byte limits are
enforced afterward. With an existing base64 input, this boundary alone costs:

| Raw file size | Median time | Allocated per call |
| --- | ---: | ---: |
| 1 MiB | 2.892 ms | 6.684 MiB |
| 8 MiB | 22.121 ms | 54.819 MiB |
| 16 MiB | 43.421 ms | 112.770 MiB |

The allocation profile attributes the cost to JSON decoder buffer growth,
marshal output cloning, and construction of the new string. Transport parsing,
decoded file allocation, multipart work, and Meta latency are excluded, so this
is only one component of upload cost.

Candidate: a typed upload decoder that reads validated string fields directly,
rejects excess encoded size before copying, and preserves unknown-field
rejection, caller authorization, MIME validation, and credential redaction.
Multipart framing already uses `io.MultiReader`; it does not make another full
copy of the decoded file. Avoid replacing that existing streaming behavior.

### 6. Conversation listing recomputes aggregates and performs preview queries

[`ListChats`](../inbox.go#L699) groups the archive before applying the page limit
and then reads each returned conversation's last message separately. Listing
100 conversations costs 1.882 ms for 1,000 total messages, 2.790 ms for 10,000, and
13.589 ms for 100,000, with about 0.24 MiB allocated per page.
[`useInboxPages`](../ui/src/use-inbox.ts#L53) refreshes every five seconds and
rereads loaded pages to propagate deletion, expiry, and retention changes.

Remove preview-query fan-out first and measure again. If aggregates still
dominate the target workload, consider transactional chat summaries with updates
on pruning, edits, expiry, send-time correction, clear, and account changes. A
TTL cache that can display deleted or expired bodies is unsuitable. At the default
10,000-row limit, the measured listing cost is relatively small; prioritize the
receipt, pagination, and upload findings before adding summary infrastructure.

## Concurrency and dependency risks requiring load measurements

These findings follow from code and were not quantified by the serial benchmarks:

- [`handleCloudWebhook`](../cloud_webhook.go#L157) launches one storage goroutine
  per verified delivery. Its eight-second response deadline does not cancel the
  plain inbox mutex wait or `BeginTx(context.Background())`. Slow storage plus
  retries can accumulate pending batches. Bound admission and queued work while
  retaining explicit commit/acknowledgement semantics; test blocked storage and
  concurrent webhook bursts before choosing a concurrency limit.
- [`storeCloudDelivery`](../cloud_delivery.go#L26) holds the manager read lock
  across the SQLite transaction. [`CloudInbox`](../sessions.go#L317) takes the
  manager write lock even for an existing inbox. [`ApplyConfig`](../sessions.go#L346)
  also trims every archive while holding that write lock, even if the retention
  limit is unchanged. Profile mixed archive reads/writes and session operations;
  first avoid unnecessary trims. Lock restructuring must preserve lifecycle and
  configuration-epoch checks. Raising the SQL connection limit alone cannot
  help while every inbox operation holds the same mutex.
- The native [`EventRouter`](../events.go#L526) awaits matching callbacks in order
  on one shared consumer, allowing a slow subscription to stall unrelated
  sessions. The Cloud router already uses `iii.VoidAction()` and does **not**
  await callback completion. In the pinned Go SDK
  `v0.24.6-rc.1.0.20261002194513-a075c6e40389`, `enqueueOutbound` does not observe
  the supplied call context while waiting on its 64-slot channel, and the
  disconnected `offline` buffer has no cap. Test stalled/disconnected local
  engines and queue drops before changing dispatch or SDK behavior.

Lower-priority candidates include clearing discarded native event slice slots
to release their payloads sooner, avoiding the second voicemail base64 decode,
and excluding already scrubbed rows from the expiration index. Their workload
impact has not been measured. Schema/regexp compilation is already cached;
additional compilation caches are unnecessary.

Cloud sends deliberately disable keep-alive and clear request replay support to
preserve the no-retry policy. Connection/TLS reuse must not be introduced without
proving ambiguous transport failures cannot cause duplicate sends. The review
does not recommend changing the existing policy.

## Reproduction and validation

From the WhatsApp Go module, use the same Go toolchain for every comparison:

```sh
go test -run '^$' -bench 'Benchmark(Inbox|DecodeCloud)' \
  -benchmem -benchtime=300ms -count=6 -cpu=1 . > /tmp/report-1.txt

go test -run '^$' -bench 'BenchmarkInboxRecordReceipt/messages=100000$' \
  -benchmem -benchtime=2s -count=1 -cpu=1 \
  -cpuprofile=/tmp/receipt.cpu -memprofile=/tmp/receipt.mem \
  -o /tmp/whatsapp-perf.test .
go tool pprof -top -focus=recordReceipt /tmp/whatsapp-perf.test /tmp/receipt.cpu

go test -run '^$' -bench 'BenchmarkDecodeCloudDocumentRequest/fileMiB=16$' \
  -benchmem -benchtime=2s -count=1 -cpu=1 \
  -memprofile=/tmp/decode.mem -o /tmp/whatsapp-perf.test .
go tool pprof -top -alloc_space -focus=decodeCloudRequest \
  /tmp/whatsapp-perf.test /tmp/decode.mem

go test ./...
go vet ./...
```

For the retention experiment, copy its `.go.txt` source into this module as
`cloud_retention_performance_test.go`, create
`/tmp/whatsapp-performance-20261005`, run
`go test -run '^TestPerformanceCloudPreviewRetention$' -count=1 -v .`, and remove
the copied file afterward. This review used a Go file overlay instead of adding
that experimental test to the normal suite. The experiment writes heap profiles
only to its temporary artifact directory and uses synthetic test configuration.

Validation passed: all 96 baseline samples, both focused profiling runs, the
isolated heap-retention experiment, `go test ./...`, and `go vet ./...`.
Production Go and UI sources are unchanged by this review. No dependencies were
installed. Use a new six-sample run and `benchstat` when an implementation changes;
repeat correctness tests for the affected lifecycle, privacy, and delivery rules.

## Storage backend change

The measurements above describe the local SQLite implementation before the
database-worker migration. The runtime worker now uses engine RPC for SQL; these
benchmark timings do not measure the remote backend. The local fixtures remain
useful for evaluating query plans. See [database-storage.md](database-storage.md)
for the new persistence contract and operational limits.
