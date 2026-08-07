/**
 * The Queues page (`#/ext/queues`): what each topic IS, who consumes it,
 * what is moving through it right now, and its failures — with the levers
 * (publish, redrive, discard) behind confirm steps because all of them
 * touch production traffic.
 *
 * The counters are the least of it. A topic's identity is its delivery
 * policy (standard vs fifo, the fifo ordering key — `harness-turn` is fifo
 * grouped by `session_id`, which is how agent turns queue — retry budget,
 * concurrency, redeliver-on-restart), its live subscribers with their own
 * retry configs, and the movement: publishes in, deliveries out, failures
 * into the DLQ. All of that is on the bus already; this page just refuses
 * to summarize it down to five numbers.
 *
 * Live without polling: a `stream` subscription on the all-spans feed
 * reloads whenever a queue function or a subscriber of the selected topic
 * executes anywhere — a publish from chat, a redrive from the CLI, a
 * consumer failing. Where nothing fires, the refresh button stands in.
 */

import {
  Badge,
  Button,
  CodeEditor,
  EmptyState,
  type Host,
  JsonHighlight,
  PageBody,
  PageHeader,
  PageShell,
  StatusDot,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  DLQ_CAPABLE,
  type DlqMessage,
  type DlqTopicInfo,
  discardOne,
  dlqMessages,
  dlqTopics,
  frameSpanNames,
  listTopics,
  publish,
  type QueueEvent,
  type QueueSetup,
  queueSetup,
  recentActivity,
  redriveAll,
  redriveOne,
  type Subscriber,
  statsForAll,
  subscribersFor,
  type TopicInfo,
  type TopicPolicy,
  type TopicStats,
  topicStats,
} from './api'

const DLQ_PAGE_SIZE = 25

/* ---------------- helpers ---------------- */

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  return kb < 1024 ? `${kb.toFixed(1)} KB` : `${(kb / 1024).toFixed(2)} MB`
}

function formatDuration(ms: number): string {
  if (ms <= 0) return 'running'
  if (ms < 1) return `${Math.round(ms * 1000)}µs`
  if (ms < 1000) return `${ms.toFixed(1)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

function formatMs(ms: number | undefined): string {
  if (ms === undefined) return '—'
  if (ms >= 60_000) return `${Math.round(ms / 60_000)}m`
  if (ms >= 1000) return `${Math.round(ms / 1000)}s`
  return `${ms}ms`
}

function ago(atMs: number, now: number): string {
  const s = Math.max(0, Math.floor((now - atMs) / 1000))
  if (s < 1) return 'now'
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  return `${Math.floor(m / 60)}h ago`
}

let feedSeq = 0

/**
 * Reload when queue traffic happens anywhere on the bus, including the
 * selected topic's subscribers executing. Debounced across a busy turn.
 */
function useQueueTraffic(
  host: Host,
  reload: () => void,
  watchedFns: readonly string[] = [],
) {
  const reloadRef = useRef(reload)
  reloadRef.current = reload
  const watchedRef = useRef(watchedFns)
  watchedRef.current = watchedFns
  const handlerId = useMemo(() => {
    feedSeq += 1
    return `iii::queue-ui::traffic-${feedSeq}`
  }, [])

  useEffect(() => {
    let timer: number | null = null
    // Trailing debounce with a ceiling: continuous traffic keeps extending a
    // plain debounce forever, and the page would never reload exactly when
    // the most is happening.
    let deadline = 0
    const offHandler = host.iii.on(handlerId, (frame: unknown) => {
      const touches = frameSpanNames(frame).some(
        (name) =>
          name.startsWith('execute engine::queue::') ||
          name === 'execute iii::durable::publish' ||
          name.startsWith('execute iii::queue::') ||
          watchedRef.current.some((fn) => name === `execute ${fn}`),
      )
      if (!touches) return
      const now = Date.now()
      if (timer === null) deadline = now + 2500
      else window.clearTimeout(timer)
      timer = window.setTimeout(
        () => {
          timer = null
          reloadRef.current()
        },
        Math.min(600, Math.max(0, deadline - now)),
      )
    })
    let offTrigger: (() => void) | undefined
    try {
      offTrigger = host.iii.registerTrigger({
        type: 'stream',
        function_id: `${handlerId}::${host.iii.browserId}`,
        config: { stream_name: 'iii:devtools:all-spans', group_id: 'all' },
      })
    } catch {
      // No stream worker: manual refresh only.
    }
    return () => {
      if (timer !== null) window.clearTimeout(timer)
      offTrigger?.()
      offHandler()
    }
  }, [host, handlerId])
}

/* ---------------- the page ---------------- */

export function QueuesPage({
  host,
  side,
  onRequestClose,
}: {
  host: Host
  side?: 'left' | 'right'
  onRequestClose?: () => void
}) {
  const [topics, setTopics] = useState<TopicInfo[] | null>(null)
  const [dlq, setDlq] = useState<DlqTopicInfo[]>([])
  const [setup, setSetup] = useState<QueueSetup>({
    adapter: 'builtin',
    policies: new Map(),
  })
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const [statsByTopic, setStatsByTopic] = useState<Map<string, TopicStats>>(
    new Map(),
  )
  const pickedRef = useRef(false)

  const load = useCallback(() => {
    Promise.all([listTopics(host), dlqTopics(host), queueSetup(host)]).then(
      async ([topicRows, dlqRows, setupValue]) => {
        setTopics(topicRows)
        setDlq(dlqRows)
        setSetup(setupValue)
        setError(null)
        // A page that opens onto nothing selected is a page that opens
        // mostly empty. Pick the topic with dead letters, else the first —
        // on the FIRST populated load only: later traffic reloads must not
        // override a deliberately cleared selection.
        if (!pickedRef.current && topicRows.length > 0) {
          pickedRef.current = true
          const dead = dlqRows[0]?.topic
          setSelected((prev) => prev ?? dead ?? topicRows[0]?.name ?? null)
        }
        setStatsByTopic(await statsForAll(host, topicRows))
      },
      (err: unknown) => setError(errorMessage(err)),
    )
  }, [host])
  useEffect(load, [load])
  useQueueTraffic(host, load)

  const dlqByTopic = useMemo(
    () => new Map(dlq.map((d) => [d.topic, d.messageCount])),
    [dlq],
  )

  const shown = useMemo(() => {
    const needle = filter.trim().toLowerCase()
    return (topics ?? []).filter(
      (t) => !needle || t.name.toLowerCase().includes(needle),
    )
  }, [topics, filter])

  const dlqTotal = dlq.reduce((n, d) => n + d.messageCount, 0)
  const volatile =
    setup.adapter === 'builtin' && setup.storeMethod !== 'file_based'

  return (
    <PageShell className="queue-ui">
      <PageHeader
        title="queues"
        description={`${topics?.length ?? '…'} topics${
          dlqTotal > 0 ? ` · ${dlqTotal} dead` : ''
        } · ${setup.adapter}${setup.storeMethod ? ` (${setup.storeMethod})` : ''}`}
        onClose={onRequestClose}
        actions={
          <>
            {volatile ? (
              <Badge
                variant="warn"
                title="the builtin store is not file-backed — queued jobs do not survive a worker restart"
              >
                volatile store
              </Badge>
            ) : null}
            <Button variant="pill" size="sm" onClick={load}>
              refresh
            </Button>
          </>
        }
      />
      <input
        className="queue-ui-filter"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        placeholder="filter topics…"
        aria-label="filter topics"
      />

      {error ? (
        <div className="queue-ui-error">
          engine::queue::list_topics failed — {error}
        </div>
      ) : topics === null ? (
        <div className="queue-ui-note">loading topics…</div>
      ) : shown.length === 0 ? (
        <EmptyState
          title={filter.trim() ? 'nothing matches' : 'no topics'}
          description={
            filter.trim()
              ? 'no topic name contains that text.'
              : 'a topic appears with its first publish or subscriber.'
          }
        />
      ) : (
        <PageBody side={side} className="queue-ui-body">
          <div className="queue-ui-list">
            <div className="queue-ui-thead" aria-hidden>
              <span className="c-name">topic</span>
              <span className="c-n">depth</span>
              <span className="c-n">delivered</span>
              <span className="c-n">failed</span>
              <span className="c-n">dead</span>
              <span className="c-n">subs</span>
            </div>
            {shown.map((topic) => {
              const dead = dlqByTopic.get(topic.name) ?? 0
              const policy = setup.policies.get(topic.name)
              const stats = statsByTopic.get(topic.name)
              return (
                <button
                  key={topic.name}
                  type="button"
                  className="queue-ui-row"
                  data-selected={selected === topic.name}
                  onClick={() =>
                    setSelected((prev) =>
                      prev === topic.name ? null : topic.name,
                    )
                  }
                >
                  <span className="c-name">
                    <span className="name">{topic.name}</span>
                    {policy?.type === 'fifo' ? (
                      <span className="fifo">
                        fifo
                        {policy.messageGroupField
                          ? ` · ${policy.messageGroupField}`
                          : ''}
                      </span>
                    ) : null}
                  </span>
                  <span className="c-n">{stats?.depth ?? '—'}</span>
                  <span className="c-n">{stats?.delivered ?? '—'}</span>
                  <span className="c-n" data-alert={(stats?.failed ?? 0) > 0}>
                    {stats?.failed ?? '—'}
                  </span>
                  <span className="c-n" data-alert={dead > 0}>
                    {dead}
                  </span>
                  <span className="c-n">{topic.subscriberCount}</span>
                </button>
              )
            })}
          </div>
          {selected ? (
            <div className="queue-ui-detail">
              <TopicDetail
                host={host}
                topic={selected}
                deadCount={dlqByTopic.get(selected) ?? 0}
                policy={setup.policies.get(selected)}
                dlqCapable={DLQ_CAPABLE.has(setup.adapter)}
                adapter={setup.adapter}
                onChanged={load}
                onClose={() => setSelected(null)}
              />
            </div>
          ) : null}
        </PageBody>
      )}
    </PageShell>
  )
}

/* ---------------- topic detail ---------------- */

function TopicDetail({
  host,
  topic,
  deadCount,
  policy,
  dlqCapable,
  adapter,
  onChanged,
  onClose,
}: {
  host: Host
  topic: string
  deadCount: number
  policy?: TopicPolicy
  dlqCapable: boolean
  adapter: string
  onChanged: () => void
  onClose: () => void
}) {
  const [stats, setStats] = useState<Awaited<
    ReturnType<typeof topicStats>
  > | null>(null)
  const [subscribers, setSubscribers] = useState<Subscriber[]>([])
  const [statsError, setStatsError] = useState<string | null>(null)

  const loadDetail = useCallback(() => {
    Promise.all([topicStats(host, topic), subscribersFor(host, topic)]).then(
      ([statsValue, subs]) => {
        setStats(statsValue)
        setSubscribers(subs)
        setStatsError(null)
      },
      (err: unknown) => setStatsError(errorMessage(err)),
    )
  }, [host, topic])
  useEffect(loadDetail, [loadDetail])
  const watched = useMemo(
    () => subscribers.map((s) => s.functionId),
    [subscribers],
  )
  useQueueTraffic(host, loadDetail, watched)

  return (
    <>
      <div className="queue-ui-detail-head">
        <span className="queue-ui-detail-title">{topic}</span>
        {policy?.type ? (
          <Badge>
            {policy.type}
            {policy.messageGroupField ? ` by ${policy.messageGroupField}` : ''}
          </Badge>
        ) : null}
        <span style={{ flex: 1 }} />
        <Button variant="pill" size="sm" onClick={onClose}>
          close
        </Button>
      </div>

      {statsError ? (
        <div className="queue-ui-error">
          engine::queue::topic_stats failed — {statsError}
        </div>
      ) : stats ? (
        <div className="queue-ui-tiles">
          <Tile label="depth" value={stats.depth} />
          <Tile label="delivered" value={stats.delivered} />
          <Tile
            label="failed"
            value={stats.failed}
            alert={(stats.failed ?? 0) > 0}
          />
          <Tile label="consumers" value={stats.consumerCount} />
          <Tile
            label="dead letters"
            value={stats.dlqDepth ?? deadCount}
            alert={(stats.dlqDepth ?? deadCount) > 0}
          />
        </div>
      ) : (
        <div className="queue-ui-note">loading stats…</div>
      )}

      <Tabs
        defaultValue={deadCount > 0 ? 'dead' : 'overview'}
        className="queue-ui-tabs"
      >
        <TabsList>
          <TabsTrigger value="overview">overview</TabsTrigger>
          <TabsTrigger value="activity">activity</TabsTrigger>
          <TabsTrigger value="publish">publish</TabsTrigger>
          <TabsTrigger value="dead">
            dead letters
            {deadCount > 0 ? <Badge variant="alert">{deadCount}</Badge> : null}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="overview">
          <OverviewPanel policy={policy} subscribers={subscribers} />
        </TabsContent>
        <TabsContent value="activity">
          <ActivityPanel host={host} topic={topic} subscribers={subscribers} />
        </TabsContent>
        <TabsContent value="publish">
          <PublishPanel host={host} topic={topic} onPublished={onChanged} />
        </TabsContent>
        <TabsContent value="dead">
          {dlqCapable ? (
            <DlqPanel host={host} topic={topic} onChanged={onChanged} />
          ) : (
            <div className="queue-ui-note">
              the {adapter} adapter is pub/sub only — it keeps no dead-letter
              queue, so failed deliveries are not retained. Switch to the
              builtin or rabbitmq adapter for retry and dead-lettering.
            </div>
          )}
        </TabsContent>
      </Tabs>
    </>
  )
}

function Tile({
  label,
  value,
  alert,
}: {
  label: string
  value: number | undefined
  alert?: boolean
}) {
  return (
    <div className="queue-ui-tile" data-alert={alert && (value ?? 0) > 0}>
      <span className="label">{label}</span>
      <span className="value">{value ?? '—'}</span>
    </div>
  )
}

/* ---------------- overview: policy + subscribers ---------------- */

function OverviewPanel({
  policy,
  subscribers,
}: {
  policy?: TopicPolicy
  subscribers: readonly Subscriber[]
}) {
  return (
    <div className="queue-ui-overview">
      <span className="queue-ui-label">delivery policy</span>
      {policy ? (
        <div className="queue-ui-policy">
          <PolicyFact
            label="ordering"
            value={
              policy.type === 'fifo'
                ? `fifo${policy.messageGroupField ? ` — one at a time per ${policy.messageGroupField}` : ''}`
                : (policy.type ?? 'standard')
            }
          />
          <PolicyFact
            label="concurrency"
            value={
              policy.concurrency !== undefined
                ? String(policy.concurrency)
                : '—'
            }
          />
          <PolicyFact
            label="retries"
            value={
              policy.maxRetries !== undefined
                ? `${policy.maxRetries} × ${formatMs(policy.backoffMs)} backoff`
                : '—'
            }
          />
          <PolicyFact label="timeout" value={formatMs(policy.timeoutMs)} />
          <PolicyFact
            label="on engine restart"
            value={
              policy.redeliverOnEngineRestart === undefined
                ? '—'
                : policy.redeliverOnEngineRestart
                  ? 'redeliver in-flight messages'
                  : 'drop in-flight messages'
            }
            warn={policy.redeliverOnEngineRestart === false}
          />
        </div>
      ) : (
        <div className="queue-ui-note">
          no explicit policy — the topic runs on the adapter's defaults.
        </div>
      )}

      <span className="queue-ui-label">
        subscribers
        <span className="count">{subscribers.length}</span>
      </span>
      {subscribers.length === 0 ? (
        <div className="queue-ui-note">
          nothing consumes this topic right now. Published messages queue until
          a durable subscriber registers for it.
        </div>
      ) : (
        subscribers.map((sub) => (
          <div key={sub.functionId} className="queue-ui-subscriber">
            <span className="fn">{sub.functionId}</span>
            <span className="who">
              {sub.worker}
              {sub.maxRetries !== undefined
                ? ` · ${sub.maxRetries} retries`
                : ''}
              {sub.backoffMs !== undefined
                ? ` · ${formatMs(sub.backoffMs)} backoff`
                : ''}
              {sub.conditionFunctionId
                ? ` · if ${sub.conditionFunctionId}`
                : ''}
            </span>
          </div>
        ))
      )}
    </div>
  )
}

function PolicyFact({
  label,
  value,
  warn,
}: {
  label: string
  value: string
  warn?: boolean
}) {
  return (
    <div className="queue-ui-fact" data-warn={warn}>
      <span className="label">{label}</span>
      <span className="value">{value}</span>
    </div>
  )
}

/* ---------------- activity: publishes + deliveries ---------------- */

function ActivityPanel({
  host,
  topic,
  subscribers,
}: {
  host: Host
  topic: string
  subscribers: readonly Subscriber[]
}) {
  const [events, setEvents] = useState<QueueEvent[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(() => {
    recentActivity(host, topic, subscribers).then(
      (rows) => {
        setEvents(rows)
        setError(null)
      },
      (err: unknown) => setError(errorMessage(err)),
    )
  }, [host, topic, subscribers])
  useEffect(load, [load])
  const watched = useMemo(
    () => subscribers.map((s) => s.functionId),
    [subscribers],
  )
  useQueueTraffic(host, load, watched)

  if (error) {
    return (
      <div className="queue-ui-error">
        engine::traces::list failed — {error}
      </div>
    )
  }
  if (events === null)
    return <div className="queue-ui-note">reading recent movement…</div>
  if (events.length === 0) {
    return (
      <div className="queue-ui-note">
        no recorded movement. Publishes onto <code>{topic}</code> and deliveries
        into its subscribers appear here as they happen.
      </div>
    )
  }

  const now = Date.now()
  return (
    <div className="queue-ui-activity">
      {events.map((event, i) => (
        <div
          key={`${event.kind}-${event.functionId}-${event.atMs}-${i}`}
          className="queue-ui-event"
        >
          <StatusDot tone={event.ok ? 'accent' : 'alert'} />
          <span className="kind" data-kind={event.kind}>
            {event.kind === 'publish' ? '→ in' : 'out →'}
          </span>
          <span className="fn">
            {event.kind === 'publish' ? 'publish' : event.functionId}
          </span>
          <span className="who">{event.worker}</span>
          <span className="when">
            {formatDuration(event.durationMs)} · {ago(event.atMs, now)}
          </span>
        </div>
      ))}
    </div>
  )
}

/* ---------------- publish ---------------- */

function PublishPanel({
  host,
  topic,
  onPublished,
}: {
  host: Host
  topic: string
  onPublished: () => void
}) {
  const [body, setBody] = useState('{\n  "test": true\n}')
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null)

  const send = async () => {
    let data: unknown
    try {
      data = JSON.parse(body)
    } catch (err) {
      setNote({ ok: false, text: errorMessage(err) })
      return
    }
    setConfirming(false)
    setBusy(true)
    try {
      await publish(host, topic, data)
      setNote({ ok: true, text: `published to ${topic}` })
      onPublished()
    } catch (err) {
      setNote({ ok: false, text: errorMessage(err) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="queue-ui-publish">
      <div className="queue-ui-note">
        goes through the real queue: every subscriber of <code>{topic}</code>{' '}
        receives it, and retry / dead-letter rules apply.
      </div>
      <CodeEditor
        value={body}
        onChange={setBody}
        language="json"
        className="queue-ui-editor"
        aria-label={`message body for ${topic}`}
      />
      <div className="queue-ui-actions">
        {confirming ? (
          <>
            <Button size="sm" onClick={send} disabled={busy}>
              {busy ? 'publishing…' : 'yes, publish'}
            </Button>
            <Button
              variant="pill"
              size="sm"
              onClick={() => setConfirming(false)}
            >
              cancel
            </Button>
          </>
        ) : (
          <Button size="sm" onClick={() => setConfirming(true)} disabled={busy}>
            publish message
          </Button>
        )}
        {note ? (
          <span className={note.ok ? 'queue-ui-ok' : 'queue-ui-bad'}>
            {note.text}
          </span>
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- dead letters ---------------- */

function DlqPanel({
  host,
  topic,
  onChanged,
}: {
  host: Host
  topic: string
  onChanged: () => void
}) {
  const [rows, setRows] = useState<DlqMessage[] | null>(null)
  const [page, setPage] = useState(0)
  const [open, setOpen] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirmingAll, setConfirmingAll] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(() => {
    dlqMessages(host, topic, page * DLQ_PAGE_SIZE, DLQ_PAGE_SIZE).then(
      (messages) => {
        setRows(messages)
        setError(null)
      },
      (err: unknown) => setError(errorMessage(err)),
    )
  }, [host, topic, page])
  useEffect(load, [load])
  useQueueTraffic(host, load)

  // The same failure repeated 40 times is one fact, not 40: lead with the
  // grouped error lines so the operator reads causes before instances.
  const grouped = useMemo(() => {
    const byError = new Map<string, number>()
    for (const row of rows ?? []) {
      const key = row.error ?? '(no error text)'
      byError.set(key, (byError.get(key) ?? 0) + 1)
    }
    return [...byError.entries()].sort((a, b) => b[1] - a[1])
  }, [rows])

  const act = async (
    label: string,
    fn: () => Promise<unknown>,
    id: string | null,
  ) => {
    setBusyId(id ?? '__all')
    try {
      await fn()
      setNote(label)
      load()
      onChanged()
    } catch (err) {
      setNote(errorMessage(err))
    } finally {
      setBusyId(null)
      setConfirmingAll(false)
    }
  }

  if (error) {
    return (
      <div className="queue-ui-error">
        engine::queue::dlq_messages failed — {error}
      </div>
    )
  }
  if (rows === null)
    return <div className="queue-ui-note">loading dead letters…</div>
  if (rows.length === 0 && page === 0) {
    return (
      <div className="queue-ui-note">
        no dead letters on <code>{topic}</code> — failed deliveries land here
        after their retries are spent.
      </div>
    )
  }

  return (
    <div className="queue-ui-dlq">
      {grouped.length > 1 ? (
        <div className="queue-ui-grouped">
          {grouped.map(([message, count]) => (
            <div key={message} className="queue-ui-grouped-row">
              <span className="count">{count}×</span>
              <span className="err">{message}</span>
            </div>
          ))}
        </div>
      ) : null}

      <div className="queue-ui-actions">
        {confirmingAll ? (
          <>
            <Button
              size="sm"
              onClick={() =>
                act('redrove all messages', () => redriveAll(host, topic), null)
              }
              disabled={busyId !== null}
            >
              yes, redrive all
            </Button>
            <Button
              variant="pill"
              size="sm"
              onClick={() => setConfirmingAll(false)}
            >
              cancel
            </Button>
          </>
        ) : (
          <Button size="sm" onClick={() => setConfirmingAll(true)}>
            redrive all
          </Button>
        )}
        {note ? <span className="queue-ui-note-inline">{note}</span> : null}
      </div>

      {rows.map((message) => (
        <div
          key={message.id}
          className="queue-ui-dead"
          data-open={open === message.id}
        >
          <button
            type="button"
            className="queue-ui-dead-head"
            onClick={() =>
              setOpen((prev) => (prev === message.id ? null : message.id))
            }
          >
            <StatusDot tone="alert" />
            <span className="err">{message.error ?? '(no error text)'}</span>
            <span className="meta">
              {message.retries !== undefined
                ? `${message.retries} retries · `
                : ''}
              {message.failedAtMs
                ? new Date(message.failedAtMs).toLocaleTimeString()
                : ''}
              {message.sizeBytes !== undefined
                ? ` · ${formatBytes(message.sizeBytes)}`
                : ''}
            </span>
          </button>
          {open === message.id ? (
            <div className="queue-ui-dead-body">
              <JsonHighlight
                code={JSON.stringify(message.payload ?? null, null, 2)}
                className="queue-ui-json"
                wrap
              />
              <div className="queue-ui-actions">
                <Button
                  size="sm"
                  disabled={busyId !== null}
                  onClick={() =>
                    act(
                      'message redriven',
                      () => redriveOne(host, topic, message.id),
                      message.id,
                    )
                  }
                >
                  {busyId === message.id ? 'working…' : 'redrive'}
                </Button>
                <Button
                  variant="pill"
                  size="sm"
                  disabled={busyId !== null}
                  onClick={() =>
                    act(
                      'message discarded',
                      () => discardOne(host, topic, message.id),
                      message.id,
                    )
                  }
                >
                  discard
                </Button>
                <span className="queue-ui-id">{message.id}</span>
              </div>
            </div>
          ) : null}
        </div>
      ))}

      <div className="queue-ui-actions">
        <Button
          variant="pill"
          size="sm"
          disabled={page === 0}
          onClick={() => setPage((p) => Math.max(0, p - 1))}
        >
          newer
        </Button>
        <Button
          variant="pill"
          size="sm"
          disabled={rows.length < DLQ_PAGE_SIZE}
          onClick={() => setPage((p) => p + 1)}
        >
          older
        </Button>
        <span className="queue-ui-note-inline">page {page + 1}</span>
      </div>
    </div>
  )
}
