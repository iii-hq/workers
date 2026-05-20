/**
 * iii-browser-sdk + harness fanout. Permissions live in the harness's
 * iii-permissions.yaml; the console only ships the mode.
 */

import { parseCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import { newMessageId } from '@/lib/session-id'
import type { Mode, ModelId } from '@/types/chat'
import type {
  AgentEvent,
  AgentMessage,
  SessionEventEnvelope,
} from '@/types/iii-agent-event'
import { createTurnStateTranslator, translateAgentEvent } from './translate'
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

  const off = client.on<SessionEventEnvelope>('ui::session::event', (env) => {
    if (!env || env.session_id !== sessionId || !env.event) return
    queue.push(env.event)
    wake()
  })

  const onAbort = () => wake()
  signal?.addEventListener('abort', onAbort, { once: true })

  let subscribed = false

  try {
    await client.call('ui::subscribe', {
      browser_id: client.browserId,
      session_id: sessionId,
    })
    subscribed = true

    const turnStateTranslator = createTurnStateTranslator()

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
    client
      .call('harness::call', {
        function_id: 'run::start',
        session_id: sessionId,
        message_id: messageId,
        payload: {
          session_id: sessionId,
          message_id: messageId,
          provider,
          model: modelId,
          mode,
          messages: [
            ...(opts?.history ?? []),
            {
              role: 'user',
              content: [{ type: 'text', text: prompt }],
              timestamp: Date.now(),
            },
          ],
        },
      })
      .catch((err) => {
        kickoffError = err instanceof Error ? err : new Error(String(err))
        if (import.meta.env.DEV) {
          console.warn('[real-backend] harness::call run::start failed', err)
        }
        wake()
      })

    while (true) {
      if (signal?.aborted) return
      if (kickoffError) {
        const err = kickoffError as Error
        yield {
          kind: 'assistant-token',
          token: `harness::call run::start failed — ${err.message}`,
        }
        yield { kind: 'assistant-end' }
        return
      }
      while (queue.length === 0 && !kickoffError && !signal?.aborted) {
        await new Promise<void>((resolve) => {
          resolveNext = resolve
        })
      }
      if (signal?.aborted) return
      if (kickoffError) continue
      const event = queue.shift()
      if (!event) continue
      const streamEvents =
        event.type === 'turn_state_changed'
          ? turnStateTranslator(event, sessionId)
          : translateAgentEvent(event, sessionId)
      for (const streamEvent of streamEvents) {
        yield streamEvent
      }
      if (event.type === 'agent_end') return
    }
  } finally {
    signal?.removeEventListener('abort', onAbort)
    off()
    if (subscribed) {
      await client
        .call('ui::unsubscribe', {
          browser_id: client.browserId,
          session_id: sessionId,
        })
        .catch(() => {})
    }
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

async function realCompactSession(
  sessionId: string,
  model: ModelId,
  history?: AgentMessage[],
  contextWindow?: number,
): Promise<CompactResult> {
  const { provider, model: modelId } = resolveRunParams(model)
  const client = await getIiiClient()
  try {
    // Reconcile session-tree with the UI's history before compacting so a
    // stale tree (only the first turn mirrored) doesn't yield a spurious
    // 'empty' when the UI has plenty to summarise. No-op when the tree
    // already has equal-or-more entries.
    let reconcileFailed = false
    if (history && history.length > 0) {
      await client
        .call('session-tree::ensure', { session_id: sessionId })
        .catch(() => {})
      await client
        .call('session-tree::reconcile', {
          session_id: sessionId,
          state_snapshot: history,
        })
        .catch((err) => {
          // A failed reconcile can yield a spurious 'empty' from a stale tree.
          reconcileFailed = true
          if (import.meta.env.DEV) {
            console.warn(
              '[compact_session] reconcile failed; compacting against current session-tree',
              err,
            )
          }
        })
    }

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
    // Empty + reconcileFailed → likely stale tree; tell the user to retry.
    const surfaceEmpty = (): CompactResult =>
      reconcileFailed
        ? {
            status: 'error',
            message:
              'compact: could not sync session history to server; retry /compact',
          }
        : { status: 'empty' }

    if (resp?.status === 'ok') {
      const tokensBefore =
        typeof resp.tokens_before === 'number' ? resp.tokens_before : 0
      // Surface zero-token "ok" as semantic empty.
      if (tokensBefore === 0) return surfaceEmpty()
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
      // Accepts both `message` and (legacy) `reason` during rollout.
      const wire = resp as { message?: unknown; reason?: unknown }
      const message =
        typeof wire.message === 'string'
          ? wire.message
          : typeof wire.reason === 'string'
            ? wire.reason
            : 'unknown summariser error'
      return { status: 'overflow', message }
    }
    if (resp?.status === 'empty') return surfaceEmpty()
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
  compactSession: realCompactSession,
}
