/**
 * The harness's auto-compact reads (chat-projects phase 3, contract C):
 * - `harness::autocompact::models {}`: the models that ran a turn on this
 *   engine in the last 7 days, with their window and `usable` (D165);
 * - `harness::autocompact::log {limit?}`: the last auto and inline
 *   compactions, newest first (D166).
 *
 * Both are internal and denied to agents; the settings dialog reads them
 * when it opens. Their presence is phase 3's detection (D168,
 * `ChatCapabilities.autocompact`). The settings are written through
 * `harness::chat-settings::set` (`./projects.ts`).
 */
import {
  type AutocompactRule,
  type CompactionTrigger,
  parseRule,
  parseSnapshotAutocompact,
  type SnapshotAutocompact,
} from '@/lib/autocompact'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'

export const AUTOCOMPACT_MODELS_FUNCTION_ID = 'harness::autocompact::models'
export const AUTOCOMPACT_LOG_FUNCTION_ID = 'harness::autocompact::log'
/** The log's default length (the harness keeps a ring of 50). */
export const AUTOCOMPACT_LOG_LIMIT = 20

export interface AutocompactModel {
  provider: string
  model: string
  /** The router catalog's name, else the model id. */
  name: string
  /** The model's window, when the catalog has it. */
  context_window: number | null
  /** The input budget the context-manager measured: the model's own limit. */
  usable: number
  last_used_ms: number
}

export interface AutocompactLogEntry {
  session_id: string
  /** The conversation's title when it compacted. */
  title: string
  at_ms: number
  trigger: CompactionTrigger
  threshold_tokens?: number
  context_before?: number
  context_after?: number
  rule: AutocompactRule | null
}

function rows(raw: unknown, keys: readonly string[]): unknown[] {
  if (Array.isArray(raw)) return raw
  if (raw && typeof raw === 'object') {
    for (const key of keys) {
      const value = (raw as Record<string, unknown>)[key]
      if (Array.isArray(value)) return value
    }
  }
  return []
}

function positive(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : undefined
}

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

/** The models out of untrusted wire JSON, the longest `usable` last. */
export function parseAutocompactModels(raw: unknown): AutocompactModel[] {
  return rows(raw, ['models'])
    .flatMap((row): AutocompactModel[] => {
      if (!row || typeof row !== 'object') return []
      const r = row as Record<string, unknown>
      const model = text(r.model)
      const usable = positive(r.usable)
      if (!model || usable === undefined) return []
      return [
        {
          provider: text(r.provider) ?? '',
          model,
          name: text(r.name) ?? model,
          context_window: positive(r.context_window) ?? null,
          usable,
          last_used_ms: positive(r.last_used_ms) ?? 0,
        },
      ]
    })
    .sort((a, b) => a.usable - b.usable || a.name.localeCompare(b.name))
}

const TRIGGERS: readonly CompactionTrigger[] = [
  'command',
  'auto',
  'inline',
  'limit',
]

/** The log out of untrusted wire JSON, newest first. */
export function parseAutocompactLog(raw: unknown): AutocompactLogEntry[] {
  return rows(raw, ['entries', 'log'])
    .flatMap((row): AutocompactLogEntry[] => {
      if (!row || typeof row !== 'object') return []
      const r = row as Record<string, unknown>
      const sessionId = text(r.session_id)
      if (!sessionId) return []
      const entry: AutocompactLogEntry = {
        session_id: sessionId,
        title: text(r.title) ?? sessionId,
        at_ms: positive(r.at_ms) ?? 0,
        trigger: TRIGGERS.find((t) => t === r.trigger) ?? 'auto',
        rule: parseRule(r.rule),
      }
      const threshold = positive(r.threshold_tokens)
      if (threshold !== undefined) entry.threshold_tokens = threshold
      const before = positive(r.context_before)
      if (before !== undefined) entry.context_before = before
      const after = positive(r.context_after)
      if (after !== undefined) entry.context_after = after
      return [entry]
    })
    .sort((a, b) => b.at_ms - a.at_ms)
}

async function client(provided?: IiiClient): Promise<IiiClient> {
  return provided ?? (await getIiiClient())
}

export async function listAutocompactModels(
  provided?: IiiClient,
): Promise<AutocompactModel[]> {
  const c = await client(provided)
  return parseAutocompactModels(
    await c.call(AUTOCOMPACT_MODELS_FUNCTION_ID, {}),
  )
}

export async function listAutocompactLog(
  limit = AUTOCOMPACT_LOG_LIMIT,
  provided?: IiiClient,
): Promise<AutocompactLogEntry[]> {
  const c = await client(provided)
  return parseAutocompactLog(
    await c.call(AUTOCOMPACT_LOG_FUNCTION_ID, { limit }),
  )
}

/* ── a running turn's snapshot ──────────────────────────────────────── */

/** The state scope the harness writes each step's context snapshot to. */
const CONTEXT_SCOPE = 'harness_context'
const STATE_ON_CHANGED_FN = 'state::on-changed'
/** `iii::`, so the delivery spans read as internal and stay out of Traces. */
const SNAPSHOT_HANDLER = 'iii::console::context_snapshot'

type SnapshotClient = Pick<IiiClient, 'browserId' | 'on' | 'observe'>

/**
 * Observe a session's context snapshot (`harness_context/<session id>`):
 * the harness writes it after every generate step, with the auto-compact
 * the turn froze (§3.2.3), so the meter agrees with the harness and the
 * injected chip while a turn runs. `onValue` gets each write's
 * `autocompact`; a write without one (an older harness) is skipped.
 * Returns a cleanup.
 */
export function startTurnAutocompactSubscription(
  client: SnapshotClient,
  sessionId: string,
  onValue: (value: SnapshotAutocompact) => void,
): () => void {
  const off = client.on(SNAPSHOT_HANDLER, (payload: unknown) => {
    if (!payload || typeof payload !== 'object') return
    // The handler id is shared by every chat's observer (client.on fans
    // out), so another session's writes land here too.
    const event = payload as {
      key?: unknown
      event_type?: unknown
      new_value?: unknown
    }
    if (event.key !== sessionId || event.event_type === 'state:deleted') return
    const snapshot = event.new_value as { session_id?: unknown } | null
    if (!snapshot || typeof snapshot !== 'object') return
    if (snapshot.session_id !== undefined && snapshot.session_id !== sessionId)
      return
    const value = parseSnapshotAutocompact(
      (snapshot as { autocompact?: unknown }).autocompact,
    )
    if (value) onValue(value)
  })
  const offObserver = client.observe(
    {
      function_id: STATE_ON_CHANGED_FN,
      filters: {
        'input.scope': { $eq: CONTEXT_SCOPE },
        'input.key': { $eq: sessionId },
      },
    },
    `${SNAPSHOT_HANDLER}::${client.browserId}`,
  )
  return () => {
    off()
    try {
      offObserver()
    } catch {
      // The SDK is already disposed.
    }
  }
}

/** `startTurnAutocompactSubscription` over the page's client. */
export function onTurnAutocompact(
  sessionId: string,
  onValue: (value: SnapshotAutocompact) => void,
): () => void {
  let disposed = false
  let off: (() => void) | null = null
  getIiiClient()
    .then((client) => {
      if (disposed) return
      off = startTurnAutocompactSubscription(client, sessionId, onValue)
    })
    .catch(() => {})
  return () => {
    disposed = true
    off?.()
  }
}
