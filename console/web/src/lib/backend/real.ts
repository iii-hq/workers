/**
 * iii-browser-sdk + harness fanout. Permissions live in the harness's
 * iii-permissions.yaml; the console only ships the mode.
 */

import { parseCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import { newMessageId } from '@/lib/session-id'
import type { Mode, ModelId } from '@/types/chat'
import type { AgentEvent } from '@/types/iii-agent-event'
import { startSessionEventsSubscription } from './session-events-live'
import { createAgentEventTranslator } from './translate'
import type {
  ChatBackend,
  ChatStreamOptions,
  CompactResult,
  StreamEvent,
} from './types'

interface RunParams {
  provider: string
  model: string
}

function resolveRunParams(model: ModelId): RunParams {
  const parsed = parseCatalogModelKey(model)
  if (parsed) {
    return { provider: parsed.provider, model: parsed.id }
  }
  const provider = model.startsWith('claude')
    ? 'anthropic'
    : model.startsWith('gemini')
      ? 'google'
      : 'openai'
  return { provider, model }
}

async function* realStream(
  prompt: string,
  mode: Mode,
  model: ModelId,
  opts?: ChatStreamOptions,
): AsyncGenerator<StreamEvent> {
  const signal = opts?.signal
  const client = await getIiiClient()
  const sessionId = opts?.sessionId ?? `console-${crypto.randomUUID()}`
  const messageId = newMessageId()

  const queue: AgentEvent[] = []
  let resolveNext: (() => void) | null = null
  const wake = () => {
    const r = resolveNext
    resolveNext = null
    r?.()
  }

  // Subscribe directly to this session's `agent::events` stream via a scoped
  // engine stream trigger (`group_id = sessionId`), replacing the harness
  // fanout hop (`ui::subscribe` → per-browser `ui::session::event` push).
  // Registered before the `harness::trigger` kickoff below — both travel the
  // same ordered WS connection, so the trigger is in place before the turn's
  // first event is written.
  const stopSubscription = startSessionEventsSubscription(
    client,
    sessionId,
    (event) => {
      queue.push(event)
      wake()
    },
  )

  const onAbort = () => wake()
  signal?.addEventListener('abort', onAbort, { once: true })

  try {
    const { translate } = createAgentEventTranslator()

    client
      .call<Record<string, unknown> | null>('turn::get_state', {
        session_id: sessionId,
      })
      .then((record) => {
        if (!record) return
        queue.push({
          type: 'turn_state_changed',
          event_type: 'state:created',
          new_value: record,
        })
        wake()
      })
      .catch((err) => {
        if (import.meta.env.DEV) {
          console.warn('[real-backend] turn::get_state recovery failed', err)
        }
      })

    const { provider, model: modelId } = resolveRunParams(model)

    let kickoffError: Error | null = null
    let sessionBusy = false
    client
      .call<{ status_code?: number; body?: { started?: boolean } }>(
        'harness::trigger',
        {
          session_id: sessionId,
          message_id: messageId,
          payload: {
            session_id: sessionId,
            message_id: messageId,
            provider,
            model: modelId,
            mode,
            messages: [
              {
                role: 'user',
                content: [{ type: 'text', text: prompt }],
                timestamp: Date.now(),
              },
            ],
          },
        },
      )
      .then((res) => {
        // run::start refused: a turn is still in flight for this session and
        // the message was NOT appended. Surface it instead of waiting on
        // events that may never come (e.g. a turn parked on an approval).
        if (res?.body?.started === false) {
          sessionBusy = true
          wake()
        }
      })
      .catch((err) => {
        kickoffError = err instanceof Error ? err : new Error(String(err))
        if (import.meta.env.DEV) {
          console.warn('[real-backend] harness::trigger failed', err)
        }
        wake()
      })

    while (true) {
      if (signal?.aborted) return
      if (sessionBusy) {
        yield {
          kind: 'stop-reason',
          reason: 'error',
          message:
            'A turn is still running in this session — your message was not sent. Stop the current turn (or answer its pending approval), then resend.',
        }
        yield { kind: 'assistant-end' }
        return
      }
      if (kickoffError) {
        const err = kickoffError as Error
        yield {
          kind: 'assistant-token',
          token: `harness::trigger failed — ${err.message}`,
        }
        yield { kind: 'assistant-end' }
        return
      }
      while (
        queue.length === 0 &&
        !kickoffError &&
        !sessionBusy &&
        !signal?.aborted
      ) {
        await new Promise<void>((resolve) => {
          resolveNext = resolve
        })
      }
      if (signal?.aborted) return
      if (kickoffError || sessionBusy) continue
      const event = queue.shift()
      if (!event) continue
      const streamEvents = translate(event, sessionId)
      for (const streamEvent of streamEvents) {
        yield streamEvent
      }
      if (event.type === 'agent_end') return
    }
  } finally {
    signal?.removeEventListener('abort', onAbort)
    stopSubscription()
  }
}

async function realResolveApproval(
  sessionId: string,
  functionCallId: string,
  decision: 'allow' | 'deny',
): Promise<void> {
  const client = await getIiiClient()
  await client.call('approval::resolve', {
    session_id: sessionId,
    function_call_id: functionCallId,
    decision,
  })
}

async function realAbortRun(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.call('run::abort', { session_id: sessionId })
}

async function realCompactSession(
  sessionId: string,
  model: ModelId,
  contextWindow?: number,
): Promise<CompactResult> {
  const { provider, model: modelId } = resolveRunParams(model)
  const client = await getIiiClient()
  try {
    // Passing limit.context lets the server skip the models::get lookup.
    // We don't know max_output here; 4096 is the same conservative default
    // the server falls back to when models::get returns nothing.
    const DEFAULT_MAX_OUTPUT = 4_096
    const modelPayload: {
      id: string
      providerID: string
      limit?: { context: number; input: number; output: number }
    } = { id: modelId, providerID: provider }
    if (typeof contextWindow === 'number' && contextWindow > 0) {
      modelPayload.limit = {
        context: contextWindow,
        input: contextWindow,
        output: DEFAULT_MAX_OUTPUT,
      }
    }

    const resp = await client.call<{
      status?: string
      tail_start_id?: string | null
      tokens_before?: number
      auto_continued?: boolean
      summary_text?: string
      message?: string
      reason?: string
    }>('context-compaction::compact_session', {
      session_id: sessionId,
      model: modelPayload,
    })
    if (resp?.status === 'ok') {
      const tokensBefore =
        typeof resp.tokens_before === 'number' ? resp.tokens_before : 0
      // Surface zero-token "ok" as semantic empty.
      if (tokensBefore === 0) return { status: 'empty' }
      // Fallback placeholder for engines that predate summary_text on the
      // wire; without it the marker has no <conversation-summary> to ship.
      const summaryText =
        typeof resp.summary_text === 'string' && resp.summary_text.length > 0
          ? resp.summary_text
          : '[prior conversation compacted by the engine]'
      return {
        status: 'ok',
        tokensBefore,
        autoContinued: Boolean(resp.auto_continued),
        summaryText,
      }
    }
    if (resp?.status === 'busy') return { status: 'busy' }
    if (resp?.status === 'overflow') {
      const message =
        typeof resp.message === 'string'
          ? resp.message
          : 'unknown summariser error'
      return { status: 'overflow', message }
    }
    if (resp?.status === 'empty') return { status: 'empty' }
    return {
      status: 'error',
      message: `unexpected status: ${String(resp?.status ?? 'null')}`,
    }
  } catch (err) {
    return {
      status: 'error',
      message: err instanceof Error ? err.message : String(err),
    }
  }
}

export const realBackend: ChatBackend = {
  id: 'real',
  stream: realStream,
  resolveApproval: realResolveApproval,
  abortRun: realAbortRun,
  compactSession: realCompactSession,
}
