/**
 * The queue worker's own read/write surface, as the console page consumes
 * it. Everything here already exists on the bus (`queue/src/functions.rs`);
 * this file only names the wire shapes and narrows unknown JSON.
 *
 * Two engine quirks worth the comment: `list_topics` and `dlq_topics`
 * answer with BARE ARRAYS, not `{topics: […]}` envelopes, and the write
 * functions live under `iii::queue::*` / `iii::durable::*` rather than
 * `queue::*` because they predate the namespace convention.
 */

import type { Host } from '@iii-dev/console-ui'

export interface TopicInfo {
  name: string
  brokerType: string
  subscriberCount: number
}

export interface TopicStats {
  depth?: number
  delivered?: number
  failed?: number
  consumerCount?: number
  dlqDepth?: number
  config?: unknown
}

export interface DlqTopicInfo {
  topic: string
  messageCount: number
}

export interface DlqMessage {
  id: string
  error?: string
  failedAtMs?: number
  retries?: number
  sizeBytes?: number
  payload?: unknown
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

const num = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v) ? v : undefined

export async function listTopics(host: Host): Promise<TopicInfo[]> {
  const out = await host.iii.trigger('engine::queue::list_topics', {})
  if (!Array.isArray(out)) return []
  return out
    .map((row): TopicInfo | null => {
      if (!isRecord(row) || typeof row.name !== 'string') return null
      return {
        name: row.name,
        brokerType: typeof row.broker_type === 'string' ? row.broker_type : '',
        subscriberCount: num(row.subscriber_count) ?? 0,
      }
    })
    .filter((t): t is TopicInfo => t !== null)
    .sort((a, b) => a.name.localeCompare(b.name))
}

export async function topicStats(
  host: Host,
  topic: string,
): Promise<TopicStats> {
  const out = await host.iii.trigger('engine::queue::topic_stats', { topic })
  if (!isRecord(out)) return {}
  return {
    depth: num(out.depth),
    delivered: num(out.delivered),
    failed: num(out.failed),
    consumerCount: num(out.consumer_count),
    dlqDepth: num(out.dlq_depth),
    config: out.config,
  }
}

export async function dlqTopics(host: Host): Promise<DlqTopicInfo[]> {
  const out = await host.iii.trigger('engine::queue::dlq_topics', {})
  if (!Array.isArray(out)) return []
  return out
    .map((row): DlqTopicInfo | null => {
      if (!isRecord(row) || typeof row.topic !== 'string') return null
      return { topic: row.topic, messageCount: num(row.message_count) ?? 0 }
    })
    .filter((t): t is DlqTopicInfo => t !== null)
}

export async function dlqMessages(
  host: Host,
  topic: string,
  offset: number,
  limit: number,
): Promise<DlqMessage[]> {
  const out = await host.iii.trigger('engine::queue::dlq_messages', {
    topic,
    offset,
    limit,
  })
  const rows = Array.isArray(out)
    ? out
    : isRecord(out) && Array.isArray(out.messages)
      ? out.messages
      : []
  return rows
    .map((row): DlqMessage | null => {
      if (!isRecord(row) || typeof row.id !== 'string') return null
      return {
        id: row.id,
        error: typeof row.error === 'string' ? row.error : undefined,
        failedAtMs: num(row.failed_at),
        retries: num(row.retries),
        sizeBytes: num(row.size_bytes),
        payload: row.payload,
      }
    })
    .filter((m): m is DlqMessage => m !== null)
}

export async function publish(
  host: Host,
  topic: string,
  data: unknown,
): Promise<void> {
  await host.iii.trigger('iii::durable::publish', { topic, data })
}

/** Returns how many messages went back onto the main queue. */
export async function redriveAll(host: Host, queue: string): Promise<number> {
  const out = await host.iii.trigger('iii::queue::redrive', { queue })
  return isRecord(out) ? (num(out.redriven) ?? 0) : 0
}

export async function redriveOne(
  host: Host,
  queue: string,
  messageId: string,
): Promise<void> {
  await host.iii.trigger('iii::queue::redrive_message', {
    queue,
    message_id: messageId,
  })
}

export async function discardOne(
  host: Host,
  queue: string,
  messageId: string,
): Promise<void> {
  await host.iii.trigger('iii::queue::discard_message', {
    queue,
    message_id: messageId,
  })
}

/** Per-topic delivery policy, from the worker's `queue_configs` entry. */
export interface TopicPolicy {
  type?: string
  messageGroupField?: string
  concurrency?: number
  maxRetries?: number
  backoffMs?: number
  timeoutMs?: number
  pollIntervalMs?: number
  redeliverOnEngineRestart?: boolean
}

export interface QueueSetup {
  /** Transport: builtin | rabbitmq | redis. */
  adapter: string
  /** builtin only: file_based survives restarts, in_memory does not. */
  storeMethod?: string
  policies: Map<string, TopicPolicy>
}

/**
 * The worker's own configuration entry: which transport runs, whether the
 * builtin store survives restarts, and the per-topic delivery policies —
 * the substance behind the counters (fifo ordering keys, retry budgets,
 * redeliver-on-restart).
 */
export async function queueSetup(host: Host): Promise<QueueSetup> {
  const setup: QueueSetup = { adapter: 'builtin', policies: new Map() }
  try {
    const out = await host.iii.trigger('configuration::get', { id: 'queue' })
    const value = isRecord(out) ? out.value : null
    const adapter = isRecord(value) ? value.adapter : null
    if (isRecord(adapter) && typeof adapter.name === 'string') {
      setup.adapter = adapter.name
    }
    const adapterConfig = isRecord(adapter) ? adapter.config : null
    if (
      isRecord(adapterConfig) &&
      typeof adapterConfig.store_method === 'string'
    ) {
      setup.storeMethod = adapterConfig.store_method
    }
    const configs = isRecord(value) ? value.queue_configs : null
    if (isRecord(configs)) {
      for (const [topic, raw] of Object.entries(configs)) {
        if (!isRecord(raw)) continue
        setup.policies.set(topic, {
          type: typeof raw.type === 'string' ? raw.type : undefined,
          messageGroupField:
            typeof raw.message_group_field === 'string'
              ? raw.message_group_field
              : undefined,
          concurrency: num(raw.concurrency),
          maxRetries: num(raw.max_retries),
          backoffMs: num(raw.backoff_ms),
          timeoutMs: num(raw.timeout_ms),
          pollIntervalMs: num(raw.poll_interval_ms),
          redeliverOnEngineRestart:
            typeof raw.redeliver_on_engine_restart === 'boolean'
              ? raw.redeliver_on_engine_restart
              : undefined,
        })
      }
    }
  } catch {
    // Configuration worker absent: defaults stand.
  }
  return setup
}

export const DLQ_CAPABLE = new Set(['builtin', 'rabbitmq'])

/**
 * Stats for every topic in one bounded sweep — the list renders real
 * numbers per row instead of a name and a subscriber count. A handful of
 * topics is the normal case; the concurrency cap keeps a large fleet from
 * stampeding the worker.
 */
export async function statsForAll(
  host: Host,
  topics: readonly TopicInfo[],
): Promise<Map<string, TopicStats>> {
  const out = new Map<string, TopicStats>()
  const queue = [...topics]
  const workers = Array.from(
    { length: Math.min(4, queue.length) },
    async () => {
      for (;;) {
        const topic = queue.shift()
        if (!topic) return
        try {
          out.set(topic.name, await topicStats(host, topic.name))
        } catch {
          // A single failing topic must not blank the whole table.
        }
      }
    },
  )
  await Promise.all(workers)
  return out
}

/** One live consumer of a topic: who runs, under what retry budget. */
export interface Subscriber {
  functionId: string
  worker: string
  maxRetries?: number
  backoffMs?: number
  conditionFunctionId?: string
}

/**
 * Live `durable:subscriber` registrations for one topic. This is the answer
 * to "who consumes this" — the list the topic row's `N subs` count summarizes.
 */
async function durableRegistrations(host: Host): Promise<unknown[]> {
  const out = await host.iii.trigger('engine::registered-triggers::list', {
    trigger_type: 'durable:subscriber',
    include_internal: true,
  })
  return isRecord(out) && Array.isArray(out.registered_triggers)
    ? out.registered_triggers
    : []
}

function registrationTopic(row: unknown): string | null {
  if (!isRecord(row)) return null
  const config = isRecord(row.config) ? row.config : {}
  if (typeof config.queue === 'string') return config.queue
  if (typeof config.topic === 'string') return config.topic
  return null
}

/**
 * Subscriber registrations per topic. The engine's own
 * `list_topics.subscriber_count` reports connected CONSUMERS and reads 0 for
 * idle durable subscribers — registrations are what an operator means by
 * "does anything consume this topic".
 */
export async function subscriberCounts(
  host: Host,
): Promise<Map<string, number>> {
  const counts = new Map<string, number>()
  for (const row of await durableRegistrations(host)) {
    const topic = registrationTopic(row)
    if (topic !== null) counts.set(topic, (counts.get(topic) ?? 0) + 1)
  }
  return counts
}

export async function subscribersFor(
  host: Host,
  topic: string,
): Promise<Subscriber[]> {
  const rows = await durableRegistrations(host)
  const subs: Subscriber[] = []
  for (const row of rows) {
    if (!isRecord(row)) continue
    const config = isRecord(row.config) ? row.config : {}
    if (config.queue !== topic && config.topic !== topic) continue
    if (typeof row.function_id !== 'string') continue
    subs.push({
      functionId: row.function_id,
      worker: typeof row.worker_name === 'string' ? row.worker_name : 'unknown',
      maxRetries: num(config.max_retries),
      backoffMs: num(config.backoff_ms),
      conditionFunctionId:
        typeof config.condition_function_id === 'string'
          ? config.condition_function_id
          : undefined,
    })
  }
  return subs.sort((a, b) => a.functionId.localeCompare(b.functionId))
}

/** One movement on the topic: a publish in, or a delivery to a consumer. */
export interface QueueEvent {
  kind: 'publish' | 'delivery'
  functionId: string
  worker: string
  atMs: number
  durationMs: number
  ok: boolean
}

/**
 * Span names carried by one all-spans stream frame. The envelope nests as
 * `{event: {event: {data: {spans}}}}`; reading the names directly beats
 * JSON.stringify'ing the whole frame just to substring-match it.
 */
export function frameSpanNames(frame: unknown): string[] {
  if (!isRecord(frame)) return []
  const outer = isRecord(frame.event) ? frame.event : undefined
  const inner = outer && isRecord(outer.event) ? outer.event : undefined
  const data = inner && isRecord(inner.data) ? inner.data : undefined
  const spans = data && Array.isArray(data.spans) ? data.spans : []
  const names: string[] = []
  for (const span of spans) {
    if (isRecord(span) && typeof span.name === 'string') names.push(span.name)
  }
  return names
}

function spanEvents(
  out: unknown,
  kind: QueueEvent['kind'],
  topicFilter?: string,
  lenient = false,
): QueueEvent[] {
  const spans = isRecord(out) && Array.isArray(out.spans) ? out.spans : []
  const events: QueueEvent[] = []
  for (const span of spans) {
    if (!isRecord(span)) continue
    if (topicFilter !== undefined) {
      // Publishes carry the topic in the recorded input payload event, so
      // they filter exactly. Delivery inputs are the bare message data — the
      // envelope is gone — so a subscriber bound to several topics is only
      // filterable when the payload happens to carry a `topic` field:
      // lenient keeps a span unless that field names a DIFFERENT topic.
      const eventsAttr = Array.isArray(span.events) ? span.events : []
      let matches = lenient
      for (const entry of eventsAttr) {
        if (!isRecord(entry) || entry.name !== 'iii.invocation.input') continue
        for (const attr of Array.isArray(entry.attributes)
          ? entry.attributes
          : []) {
          if (!Array.isArray(attr) || attr[0] !== 'iii.payload.json') continue
          try {
            const payload = JSON.parse(String(attr[1]))
            if (!isRecord(payload)) continue
            if (payload.topic === topicFilter) {
              matches = true
            } else if (lenient && typeof payload.topic === 'string') {
              matches = false
            }
          } catch {
            // Unparseable payload: not a match.
          }
        }
      }
      if (!matches) continue
    }
    const start = Number(span.start_time_unix_nano)
    const end = Number(span.end_time_unix_nano)
    if (!Number.isFinite(start) || start <= 0) continue
    const name = typeof span.name === 'string' ? span.name : ''
    events.push({
      kind,
      functionId: name.startsWith('execute ') ? name.slice(8) : name,
      worker:
        typeof span.service_name === 'string' ? span.service_name : 'unknown',
      atMs: start / 1e6,
      durationMs: Number.isFinite(end) && end > start ? (end - start) / 1e6 : 0,
      ok: span.status !== 'error',
    })
  }
  return events
}

/**
 * Recent movement on one topic, from the trace store: publishes onto it
 * (filtered by the recorded payload's `topic`) and deliveries into each of
 * its subscribers (their execution spans). Newest first.
 */
export async function recentActivity(
  host: Host,
  topic: string,
  subscribers: readonly Subscriber[],
): Promise<QueueEvent[]> {
  const publishRead = host.iii
    .trigger('engine::traces::list', {
      name: 'execute iii::durable::publish',
      limit: 60,
      include_internal: true,
    })
    .then((out) => spanEvents(out, 'publish', topic))
    .catch((): QueueEvent[] => [])

  // Bounded like statsForAll — a topic with many subscribers must not
  // stampede the engine with one traces call each, all at once.
  const deliveries: QueueEvent[] = []
  const pending = [...subscribers]
  const pool = Array.from(
    { length: Math.min(4, pending.length) },
    async () => {
      for (;;) {
        const sub = pending.shift()
        if (!sub) return
        try {
          const out = await host.iii.trigger('engine::traces::list', {
            name: `execute ${sub.functionId}`,
            limit: 25,
            include_internal: true,
          })
          deliveries.push(...spanEvents(out, 'delivery', topic, true))
        } catch {
          // One unreadable subscriber must not blank the feed.
        }
      }
    },
  )
  await Promise.all(pool)
  return [...(await publishRead), ...deliveries]
    .sort((a, b) => b.atMs - a.atMs)
    .slice(0, 40)
}
