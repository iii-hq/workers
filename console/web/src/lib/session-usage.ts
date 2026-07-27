/**
 * Session and per-turn usage rollups, computed from the transcript the
 * console already holds. Pure: no React, no iii client, no fetching — so the
 * arithmetic (which is the part that is easy to get wrong) is unit-testable
 * on its own.
 *
 * Two rules govern everything here:
 *
 * 1. **`total = input + output`, never more.** Cache and reasoning tokens are
 *    not portably additive. Anthropic's `input` excludes cached tokens and
 *    reports them separately; OpenAI's `input` already includes `cache_read`
 *    and its `output` already includes `reasoning`. Adding them would
 *    double-count on OpenAI. This matches the convention eval already uses
 *    (`eval/src/report.rs:59-63`, `eval/src/limits.rs:128-134`).
 *
 * 2. **Absent is not zero.** Providers report disjoint subsets — codex never
 *    reports `cache_write`, anthropic never reports `reasoning`. Every total
 *    carries a `reported` count so the UI can render `—` ("not reported")
 *    rather than a confident `0`.
 *
 * Within one entry, provider usage is a cumulative running total that gets
 * overwritten as the stream progresses. Across entries (steps of a tool loop)
 * it sums — each step is a separate provider request.
 */

import type { Usage } from '@/lib/sessions/types'
import { formatTokenCount } from '@/lib/token-estimate'
import type { Message } from '@/types/chat'

export type UsageField =
  | 'input'
  | 'output'
  | 'cacheRead'
  | 'cacheWrite'
  | 'reasoning'
  | 'cost'

const USAGE_FIELDS: UsageField[] = [
  'input',
  'output',
  'cacheRead',
  'cacheWrite',
  'reasoning',
  'cost',
]

export interface UsageTotals {
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  reasoning: number
  costUsd: number
  /**
   * How many contributing entries reported each field. Zero means the field
   * was never reported and must render `—`, not `0`.
   */
  reported: Record<UsageField, number>
  /** `input + output`. Deliberately excludes cache and reasoning — see above. */
  total: number
}

export interface StepUsage {
  entryId: string
  usage: Usage
}

export interface TurnUsage {
  turnId: string
  /** Assistant entries in this turn — one per model call in the tool loop. */
  steps: number
  stepUsage: StepUsage[]
  totals: UsageTotals
  functionCalls: number
  functionCallErrors: number
  startedAt: number
  endedAt: number
  durationMs: number
  /**
   * Message id of the turn's last assistant prose segment — where the chip
   * hangs. Undefined for a tool-only turn, which has no prose to hang it on.
   */
  anchorId?: string
  streaming: boolean
}

export interface SessionUsage {
  totals: UsageTotals
  turns: TurnUsage[]
  /** Model calls (assistant entries). Not the same as turns. */
  steps: number
  /** Steps that reported no usage at all — drives the "N of M" footer. */
  stepsMissingUsage: number
  functionCalls: number
  functionCallErrors: number
  startedAt: number
  endedAt: number
  durationMs: number
  /**
   * The most recent step's raw, unsummed numbers. Used to cross-check the
   * chars/4 context estimate against something the provider actually said,
   * without doing any cross-field arithmetic on it.
   */
  lastCall?: { usage: Usage; model?: string; at: number }
}

function emptyTotals(): UsageTotals {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    costUsd: 0,
    reported: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      cost: 0,
    },
    total: 0,
  }
}

function addNumber(
  totals: UsageTotals,
  key: Exclude<keyof UsageTotals, 'reported' | 'total'>,
  field: UsageField,
  value: number | undefined,
): void {
  if (typeof value !== 'number' || !Number.isFinite(value)) return
  totals[key] += value
  totals.reported[field] += 1
}

/** Fold one entry's usage into a running total. */
export function accumulate(totals: UsageTotals, usage: Usage): void {
  addNumber(totals, 'input', 'input', usage.input)
  addNumber(totals, 'output', 'output', usage.output)
  addNumber(totals, 'cacheRead', 'cacheRead', usage.cache_read)
  addNumber(totals, 'cacheWrite', 'cacheWrite', usage.cache_write)
  addNumber(totals, 'reasoning', 'reasoning', usage.reasoning)
  addNumber(totals, 'costUsd', 'cost', usage.cost_usd)
  // The only cross-provider-safe total.
  totals.total = totals.input + totals.output
}

/** True when at least one field of `totals` was actually reported. */
export function hasReportedUsage(totals: UsageTotals): boolean {
  return USAGE_FIELDS.some((f) => totals.reported[f] > 0)
}

/** Drop a usage object that carries no usable numbers at all. */
export function normalizeUsage(usage: Usage | undefined): Usage | undefined {
  if (!usage) return undefined
  const hasAny = (
    [
      'input',
      'output',
      'cache_read',
      'cache_write',
      'reasoning',
      'cost_usd',
    ] as const
  ).some((k) => typeof usage[k] === 'number' && Number.isFinite(usage[k]))
  return hasAny ? usage : undefined
}

/**
 * Recover the turn id from a harness assistant entry id.
 *
 * The harness mints these as `e_<turn_id>_<step>_assistant`
 * (`harness/src/ids.rs:63`), and **turn ids themselves contain underscores**
 * (`t_<uuid>`), so the split has to come off the *last* separator, not the
 * first. A segment id may carry a `:<block-index>` suffix from the mapper.
 */
export function parseTurnId(messageId: string): string | undefined {
  const entryId = messageId.split(':')[0] ?? ''
  if (!entryId.startsWith('e_') || !entryId.endsWith('_assistant')) {
    return undefined
  }
  const middle = entryId.slice('e_'.length, -'_assistant'.length)
  const lastSep = middle.lastIndexOf('_')
  if (lastSep <= 0) return undefined
  const turnId = middle.slice(0, lastSep)
  return turnId.length > 0 ? turnId : undefined
}

/** A function-trigger message whose output is an error envelope. */
function isErroredTrigger(message: Message): boolean {
  if (message.role !== 'function-trigger') return false
  const output: unknown = message.output
  return typeof output === 'object' && output !== null && 'error' in output
}

interface TurnBucket {
  turnId: string
  entries: Map<string, Usage>
  order: string[]
  functionCalls: number
  functionCallErrors: number
  startedAt: number
  endedAt: number
  anchorId?: string
  streaming: boolean
}

/**
 * Assign every message the turn it belongs to.
 *
 * The mapper only stamps `turnId` on assistant entries, so a user message
 * arrives without one. It must still join the turn it *starts* — otherwise
 * every harness turn splits into two buckets (the prompt and the work) and
 * the turn count doubles. Hence the backward pass: an unkeyed user message
 * adopts the key of whatever follows it.
 *
 * Everything else falls back forward, to the turn already in progress. When
 * no key exists anywhere — optimistic local sends, `e_idem_*` webhook
 * entries, sessions written before the harness stamped origins — each user
 * message opens a synthetic `local:N` turn.
 */
function resolveTurnKeys(messages: readonly Message[]): string[] {
  const keys: (string | undefined)[] = messages.map(
    (m) => m.turnId ?? parseTurnId(m.id),
  )

  // Backward: a prompt belongs to the turn it opens.
  for (let i = messages.length - 2; i >= 0; i--) {
    if (!keys[i] && messages[i].role === 'user') keys[i] = keys[i + 1]
  }

  // Forward: continuations inherit; unkeyed prompts open a synthetic turn.
  let fallbackIndex = 0
  let previous: string | undefined
  const resolved: string[] = []
  for (let i = 0; i < messages.length; i++) {
    let key = keys[i]
    if (!key) {
      if (messages[i].role === 'user' || !previous) {
        fallbackIndex += 1
        key = `local:${fallbackIndex}`
      } else {
        key = previous
      }
    }
    resolved.push(key)
    previous = key
  }
  return resolved
}

function bucketTurns(messages: readonly Message[]): TurnBucket[] {
  const buckets: TurnBucket[] = []
  const byId = new Map<string, TurnBucket>()
  const keys = resolveTurnKeys(messages)

  const bucketFor = (turnId: string): TurnBucket => {
    let bucket = byId.get(turnId)
    if (!bucket) {
      bucket = {
        turnId,
        entries: new Map(),
        order: [],
        functionCalls: 0,
        functionCallErrors: 0,
        startedAt: Number.POSITIVE_INFINITY,
        endedAt: 0,
        streaming: false,
      }
      byId.set(turnId, bucket)
      buckets.push(bucket)
    }
    return bucket
  }

  for (const [index, message] of messages.entries()) {
    const current = bucketFor(keys[index])

    current.startedAt = Math.min(current.startedAt, message.createdAt)
    current.endedAt = Math.max(current.endedAt, message.createdAt)

    // Usage is keyed by entry so that the several segments produced from one
    // assistant entry contribute their (identical) usage exactly once.
    const usage = normalizeUsage(message.usage)
    if (usage) {
      const entryId = message.id.split(':')[0] ?? message.id
      if (!current.entries.has(entryId)) current.order.push(entryId)
      current.entries.set(entryId, usage)
    }

    if (message.role === 'function-trigger') {
      current.functionCalls += 1
      if (isErroredTrigger(message)) current.functionCallErrors += 1
    }
    if (message.role === 'assistant') {
      if (message.content.length > 0) current.anchorId = message.id
      if (message.streaming) current.streaming = true
    }
  }

  return buckets
}

function finishTurn(bucket: TurnBucket): TurnUsage {
  const totals = emptyTotals()
  const stepUsage: StepUsage[] = []
  for (const entryId of bucket.order) {
    const usage = bucket.entries.get(entryId)
    if (!usage) continue
    stepUsage.push({ entryId, usage })
    accumulate(totals, usage)
  }
  const startedAt = Number.isFinite(bucket.startedAt) ? bucket.startedAt : 0
  return {
    turnId: bucket.turnId,
    steps: stepUsage.length,
    stepUsage,
    totals,
    functionCalls: bucket.functionCalls,
    functionCallErrors: bucket.functionCallErrors,
    startedAt,
    endedAt: bucket.endedAt,
    durationMs: Math.max(0, bucket.endedAt - startedAt),
    ...(bucket.anchorId ? { anchorId: bucket.anchorId } : {}),
    streaming: bucket.streaming,
  }
}

export function turnUsages(messages: readonly Message[]): TurnUsage[] {
  return bucketTurns(messages).map(finishTurn)
}

/** Index turns by their anchor message id, for rendering the in-transcript chip. */
export function turnUsageByAnchor(
  messages: readonly Message[],
): Map<string, TurnUsage> {
  const map = new Map<string, TurnUsage>()
  for (const turn of turnUsages(messages)) {
    if (turn.anchorId) map.set(turn.anchorId, turn)
  }
  return map
}

export function sessionUsage(messages: readonly Message[]): SessionUsage {
  const turns = turnUsages(messages)
  const totals = emptyTotals()

  // Count model calls across the whole transcript rather than per turn: an
  // assistant entry that produced only tool calls still cost a request, and
  // it may not belong to any turn that has prose.
  const seenEntries = new Set<string>()
  let steps = 0
  let stepsMissingUsage = 0
  let functionCalls = 0
  let functionCallErrors = 0
  let startedAt = Number.POSITIVE_INFINITY
  let endedAt = 0
  let lastCall: SessionUsage['lastCall']

  for (const message of messages) {
    startedAt = Math.min(startedAt, message.createdAt)
    endedAt = Math.max(endedAt, message.createdAt)

    if (message.role === 'function-trigger') {
      functionCalls += 1
      if (isErroredTrigger(message)) functionCallErrors += 1
    }

    const entryId = message.id.split(':')[0] ?? message.id
    if (seenEntries.has(entryId)) continue
    const isModelCall =
      message.role === 'assistant' ||
      message.role === 'thought' ||
      message.role === 'function-trigger'
    if (!isModelCall) continue
    seenEntries.add(entryId)
    steps += 1

    const usage = normalizeUsage(message.usage)
    if (usage) {
      accumulate(totals, usage)
      lastCall = {
        usage,
        ...(message.role === 'assistant' && message.model
          ? { model: message.model }
          : {}),
        at: message.createdAt,
      }
    } else {
      stepsMissingUsage += 1
    }
  }

  return {
    totals,
    turns,
    steps,
    stepsMissingUsage,
    functionCalls,
    functionCallErrors,
    startedAt: Number.isFinite(startedAt) ? startedAt : 0,
    endedAt,
    durationMs: Number.isFinite(startedAt)
      ? Math.max(0, endedAt - startedAt)
      : 0,
    ...(lastCall ? { lastCall } : {}),
  }
}

/**
 * Value formatting, ported from eval's `formatMetric`
 * (`eval/ui/src/components.tsx:55-68`) so the two surfaces read the same.
 * Token counts delegate to the console's existing `formatTokenCount`.
 */
export function formatUsageValue(
  value: number | undefined,
  kind: 'number' | 'tokens' | 'duration' | 'cost' = 'number',
): string {
  if (value === undefined || !Number.isFinite(value)) return '—'
  switch (kind) {
    case 'tokens':
      return formatTokenCount(value)
    case 'duration':
      return value >= 1000
        ? `${(value / 1000).toFixed(2)}s`
        : `${value.toFixed(0)}ms`
    case 'cost':
      return `$${value.toFixed(6)}`
    default:
      return new Intl.NumberFormat().format(value)
  }
}

/**
 * The value for a metric row: `—` when the field was never reported, so that
 * "this provider does not report cache writes" never reads as "zero cache
 * writes".
 */
export function reportedValue(
  totals: UsageTotals,
  field: UsageField,
  value: number,
  kind: 'number' | 'tokens' | 'duration' | 'cost' = 'number',
): string {
  if (totals.reported[field] === 0) return '—'
  return formatUsageValue(value, kind)
}

/** `12m 04s` / `4h 07m` / `38s` — coarser than eval's, for session spans. */
export function formatSpan(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '—'
  const totalSeconds = Math.floor(ms / 1000)
  const seconds = totalSeconds % 60
  const minutes = Math.floor(totalSeconds / 60) % 60
  const hours = Math.floor(totalSeconds / 3600)
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, '0')}m`
  if (minutes > 0) return `${minutes}m ${String(seconds).padStart(2, '0')}s`
  return `${seconds}s`
}
