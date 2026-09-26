# Event-driven pull request monitoring

## Boundaries

GitHub monitoring uses three independently reusable worker surfaces:

1. `quick-tunnel` owns `cloudflared` processes and expiring consumer leases. It
   exposes only operator-configured local targets, not arbitrary caller URLs.
2. `http` owns HTTP ingress. Its optional webhook listener serves only routes
   explicitly registered with `public_webhook: true`. The ordinary listener,
   Console, and engine WebSocket must never be used as the public origin.
3. `github` owns repository hooks, signature verification, PR correlation,
   persistent watches, notification delivery, and cleanup.

Worker calls and trigger delivery are routed through the engine. No worker
calls another worker directly. The tunnel's HTTP forwarding is transport, not
an alternative worker RPC surface.

The webhook listener and PR monitoring are disabled by default. Persistent PR
monitoring currently requires Unix; enabling it on other platforms fails closed.
The tunnel and HTTP listener must share a network namespace because the tunnel
only accepts explicitly configured loopback origins. Merely starting
or collecting the interfaces of these workers must not open a tunnel or create
repository hooks. Existing one-shot GitHub functions remain available.

## Subscription lifecycle

A consumer chooses a unique watch ID, binds `github::pr::event` for that ID, and
then calls `github::pr::watch`. It reads the returned snapshot to cover events
that happened during setup. The watcher takes a one-time PR, checks and commit-status baseline; it does not
repeatedly query the PR while idle. Event-triggered correlation and recovery
queries are allowed and must not be described as periodic monitoring.

The provider manages one hook per installation/repository, shared by all
interested watches. It must not adopt or remove hooks belonging to other tools.
The hook's event selection covers only required event categories. A hook receives
repository-wide events; PR filtering belongs in the provider.

`pull_request` with action `closed` and `merged: true` ends a merge-only watch.
Closing without merge does not. Cancellation and an explicit expiry also end a
watch. The terminal notification must be persisted before releasing external
resources. Failure to remove a hook remains observable as pending cleanup.

The final watch releases the repository hook; the final tunnel consumer releases
the process. These are separate reference counts: GitHub cleanup must not break
another worker's public endpoint.

## Security and reliability invariants

- Verify HMAC-SHA256 over the original bytes from the HTTP request-body channel,
  not a JSON reserialization. Compare signatures in constant time.
- Treat paths, event names, headers, repository IDs, and JSON as untrusted until
  validated. Enforce byte limits and bounded execution time.
- Do not place credentials or HMAC secrets in public URLs, logs, notifications,
  status responses, or checked-in examples.
- Confirm durable acceptance before returning HTTP 2xx, within GitHub's ten
  second delivery deadline. Persisted work must survive worker restart.
- Deduplication and processing must not create a crash window in which an event
  is marked processed but its notification has not been persisted.
- Delivery is at-least-once; consumers must be idempotent. Neither GitHub order
  nor retry order implies entity-version order.
- Track the current head SHA. An old check result cannot make a new commit pass.
  A single successful check does not mean every required check passed.
- Keep failed notification attempts visible and retryable; a best-effort
  activity feed is not a substitute for a durable notification outbox.
- Configure a durable queue transport. This repository's Redis pub/sub adapter
  does not provide the durability/retry guarantee required here.
- Require explicit authorization for publishing a listener and creating hooks.
  Read access to a public repository does not grant webhook administration.
- Keep receiver, queue, and lifecycle callback functions out of the agent-facing
  callable surface; agents use watch/unwatch/status instead.

## Tunnel URL rotation and outages

A recreated Quick Tunnel may receive a new URL. Consumers subscribe to tunnel
changes before acquiring a lease and read its status once afterward. A generation
identifies a process incarnation; stale events must not overwrite the current
endpoint. URL updates preserve the existing hook secret.

GitHub does **not** automatically retry failed webhook deliveries. Startup,
reconnection, and manual recovery therefore request available failed-delivery
redeliveries and reconcile active PR snapshots. This cannot guarantee that every
historical event is recovered after an arbitrarily long outage or a silent
failure. No periodic PR polling is introduced to conceal this limitation.

Quick Tunnels are temporary and have no SLA. Production monitoring with stronger
availability requirements should use a stable hostname and an appropriate
recovery/audit policy.

## Storage migration and retention

Watches, repository hooks, inbox/outbox jobs, subscribers, publication claims,
delivery IDs and entity fingerprints use keyed SQLite tables. Mutations commit
only touched rows with `synchronous=FULL`. Copy-on-write snapshots share queued
payloads; accepting a delivery does not serialize the entire pending inbox.
Storage work runs outside the SDK's current-thread executor.

The first enabled start migrates the legacy `state` document atomically,
preserving installation identity, secrets, leases and pending work. Before
upgrading an existing installation, take a consistent SQLite backup, protect it
like the original database, and rehearse the migration on a copy. Allow disk space
for both the old document and the new tables during the transaction. **Downgrade
is not automatic:** old binaries are deliberately blocked from writing to a
migrated database. Restoring an older backup also requires reconciling GitHub
hooks and tunnel leases; do not run old and new installations concurrently.

Retention runs on the first mutation and then at most once per minute:

- Completed delivery IDs: up to seven days and the newest 100,000 entries;
  pending inbox jobs pin their delivery IDs until processing completes.
- Entity fingerprints: up to seven days and the newest 4,096 per watch.
- Cleaned-up terminal snapshots: up to seven days and the newest 1,000 eligible
  watches, with at least five minutes before capacity eviction. Pending outbox
  jobs, cleanup or tunnel leases prevent eviction.

Jobs are never removed by retention. These are periodic retention bounds, not
instantaneous hard caps: bursts and pinned work can exceed them. After snapshot
retention, `watch-status` returns not found. Redelivery beyond the deduplication
window may produce another notification, so consumers must remain idempotent.

Pending payloads are still cached in memory once; `max_pending` limits job count,
not total bytes. Size this setting together with `max_body_bytes` and available
memory. Copy-on-write index updates scale with key count rather than total queued
payload bytes. An HTTP timeout can occur while an already-started transaction
finishes: a retry is safe through delivery deduplication, but no exactly-once
guarantee is implied.

## Integration acceptance checklist

- Two watches in a repository share one managed hook.
- Comments, reviews, checks, statuses, and PR transitions route only to matching
  watches; ordinary issue comments are excluded.
- Forks and CI events with empty PR arrays can be correlated without attributing
  the result to an unrelated PR or old commit.
- Duplicate deliveries and replayed setup/recovery actions are idempotent.
- A tampered body fails signature verification, including byte-sensitive UTF-8
  and JSON-whitespace cases.
- Private routes cannot be reached through the public listener, including
  alternate methods and encoded-path variants.
- Durable state and accepted events survive restart.
- URL rotation preserves authentication and initiates recovery.
- Merging one watched PR leaves all other watches and tunnel consumers active.
- Failed deletion remains pending, with no false cleanup-success response.
- Tests use local fakes by default: no GitHub mutation or real Cloudflare tunnel.

## References

- https://try.cloudflare.com/
- https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/trycloudflare/
- https://docs.github.com/en/webhooks/using-webhooks/best-practices-for-using-webhooks
- https://docs.github.com/en/webhooks/using-webhooks/handling-failed-webhook-deliveries
- https://docs.github.com/en/rest/repos/webhooks
