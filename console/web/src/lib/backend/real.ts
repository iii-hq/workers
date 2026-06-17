/**
 * iii-browser-sdk + harness turn kickoff. Permissions live in the harness's
 * iii-permissions.yaml; the console ships the per-mode system prompt and the
 * dispatch policy on each send.
 *
 * Transcript content (tokens, message snapshots, function-call cards, results)
 * renders from session-manager events reconciled by the conversations layer
 * (lib/sessions/*). This stream carries only the ephemeral turn surface from
 * the harness/approval triggers: pending approvals (approval::pending-*),
 * stop-reason notices, and the turn-over signal (harness::turn-completed).
 */

import { parseCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import { newMessageId } from '@/lib/session-id'
import { appendCustomEntry, fetchTranscript } from '@/lib/sessions/api'
import { COMPACTION_CUSTOM_TYPE } from '@/lib/sessions/entry-mapper'
import type { AgentMessage } from '@/lib/sessions/types'
import type { Mode, ModelId } from '@/types/chat'
import {
  listPendingApprovals,
  startApprovalEventsSubscription,
} from './approval-events-live'
import {
  getTurnStatus,
  type HarnessFunctionPolicy,
  type HarnessThinkingLevel,
  isTurnActive,
  sendTurn,
  stopTurn,
} from './harness-send'
import { buildModeSystemPrompt } from './system-prompt'
import {
  isTerminalSource,
  type TurnSourceEvent,
  translateTurnSource,
} from './translate'
import { startTurnEventsSubscription } from './turn-events-live'
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

/**
 * The chat composer is a general-purpose agent surface: expose the whole bus
 * via `agent_trigger`. The harness's `iii-permissions.yaml` and the
 * approval-gate remain the safety layer (deny-by-default plumbing, human gate).
 */
const CHAT_FUNCTION_POLICY: HarnessFunctionPolicy = {
  allow: ['*'],
  expose: 'agent_trigger',
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

/** The harness omits `thinking_level` entirely when the user picks 'off'. */
function toThinkingLevel(
  level: ChatStreamOptions['thinkingLevel'],
): HarnessThinkingLevel | undefined {
  return level && level !== 'off' ? level : undefined
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
  const messageId = opts?.messageId ?? newMessageId()

  const queue: TurnSourceEvent[] = []
  let resolveNext: (() => void) | null = null
  const wake = () => {
    const r = resolveNext
    resolveNext = null
    r?.()
  }
  const push = (event: TurnSourceEvent) => {
    queue.push(event)
    wake()
  }

  // Dedupe pending approvals seen by both the catch-up read and the live
  // trigger (at-least-once, and the two can race on start).
  const seenPending = new Set<string>()
  const pushPending = (functionCallId: string, event: TurnSourceEvent) => {
    if (seenPending.has(functionCallId)) return
    seenPending.add(functionCallId)
    push(event)
  }

  const stopTurnEvents = startTurnEventsSubscription(client, sessionId, {
    onCompleted: (event) => push({ kind: 'turn-completed', event }),
  })
  const stopApprovalEvents = startApprovalEventsSubscription(
    client,
    sessionId,
    {
      onCreated: (record) =>
        pushPending(record.function_call_id, {
          kind: 'approval-created',
          record,
        }),
      onResolved: (event) => push({ kind: 'approval-resolved', event }),
    },
  )

  const onAbort = () => wake()
  signal?.addEventListener('abort', onAbort, { once: true })

  let kickoffError: Error | null = null
  try {
    const { provider, model: modelId } = resolveRunParams(model)

    // Reconnect recovery: rebuild any pending-approval cards a prior turn left
    // held (e.g. resending after a reload). Best-effort.
    void listPendingApprovals(client, sessionId)
      .then((records) => {
        for (const record of records) {
          pushPending(record.function_call_id, {
            kind: 'approval-created',
            record,
          })
        }
      })
      .catch((err) => {
        if (import.meta.env.DEV) {
          console.warn('[real-backend] approval::list-pending recovery', err)
        }
      })

    const thinkingLevel = toThinkingLevel(opts?.thinkingLevel)

    // Kick off (or steer) the turn. A turn already running for this session
    // folds the message in (merge) rather than rejecting — no busy error.
    void sendTurn(client, {
      session_id: sessionId,
      message: prompt,
      model: modelId,
      provider,
      idempotency_key: messageId,
      session: { metadata: { surface: 'console' } },
      options: {
        system_prompt: buildModeSystemPrompt(mode, provider),
        functions: CHAT_FUNCTION_POLICY,
        ...(thinkingLevel ? { thinking_level: thinkingLevel } : {}),
        metadata: { session_id: sessionId, message_id: messageId },
      },
    }).catch((err) => {
      kickoffError = err instanceof Error ? err : new Error(String(err))
      if (import.meta.env.DEV) {
        console.warn('[real-backend] harness::send failed', err)
      }
      wake()
    })

    while (true) {
      if (signal?.aborted) return
      if (kickoffError) {
        const err = kickoffError as Error
        yield {
          kind: 'stop-reason',
          reason: 'error',
          message: `harness::send failed — ${err.message}`,
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
      for (const streamEvent of translateTurnSource(event)) {
        yield streamEvent
      }
      if (isTerminalSource(event)) return
    }
  } finally {
    signal?.removeEventListener('abort', onAbort)
    stopTurnEvents()
    stopApprovalEvents()
  }
}

async function realResolveApproval(
  sessionId: string,
  functionCallId: string,
  decision: 'allow' | 'deny',
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('approval::resolve', {
    session_id: sessionId,
    function_call_id: functionCallId,
    decision,
  })
}

async function realAbortRun(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await stopTurn(client, sessionId)
}

/** `context::compact` response, discriminated on `status`. */
type CompactResponse =
  | {
      status: 'ok'
      summary: string
      tail_start_index: number | null
      tokens_before: number
      tokens_after: number
      used_prior_summary: boolean
    }
  | { status: 'busy' }
  | { status: 'empty' }
  | { status: 'overflow' }

/**
 * `/compact` — caller-owned compaction (harness.md / context-manager.md): read
 * the transcript, summarise the head via `context::compact`, then persist a
 * `compaction` custom entry the same way the harness does so the marker renders
 * and future turns anchor on it. Refuses while a turn is live.
 */
async function realCompactSession(
  sessionId: string,
  model: ModelId,
  contextWindow?: number,
): Promise<CompactResult> {
  const { provider, model: modelId } = resolveRunParams(model)
  const client = await getIiiClient()
  try {
    // Don't compact under a live turn — the harness owns the active path.
    const status = await getTurnStatus(client, sessionId).catch(() => null)
    if (status && isTurnActive(status.status)) return { status: 'busy' }

    const items = (await fetchTranscript(sessionId)).filter(
      (item): item is typeof item & { message: AgentMessage } =>
        item.message !== undefined,
    )
    if (items.length === 0) return { status: 'empty' }
    const messages = items.map((item) => item.message)

    const DEFAULT_MAX_OUTPUT = 4_096
    const modelInput: {
      id: string
      provider?: string
      limits?: { context_window: number; max_output_tokens: number }
    } = { id: modelId, provider }
    if (typeof contextWindow === 'number' && contextWindow > 0) {
      modelInput.limits = {
        context_window: contextWindow,
        max_output_tokens: DEFAULT_MAX_OUTPUT,
      }
    }

    const resp = await client.trigger<CompactResponse>('context::compact', {
      messages,
      model: modelInput,
      // Serialise concurrent compactions of the same conversation.
      options: { lease_key: sessionId },
    })

    if (resp?.status === 'ok') {
      const tailStartEntryId =
        typeof resp.tail_start_index === 'number' &&
        items[resp.tail_start_index]
          ? items[resp.tail_start_index].entry_id
          : null
      // Persist the marker the same shape the harness writes, so it renders
      // (session::message-added → conversations layer) and the next turn's
      // assemble reads `summary` + `tail_start_entry_id` to anchor.
      await appendCustomEntry({
        session_id: sessionId,
        custom_type: COMPACTION_CUSTOM_TYPE,
        data: {
          summary: resp.summary,
          tail_start_entry_id: tailStartEntryId,
          tokens_before: resp.tokens_before,
          timestamp: Date.now(),
        },
      }).catch((err) => {
        if (import.meta.env.DEV) {
          console.warn('[real-backend] persist compaction entry failed', err)
        }
      })
      return {
        status: 'ok',
        tokensBefore: resp.tokens_before,
        autoContinued: false,
        summaryText:
          resp.summary.length > 0
            ? resp.summary
            : '[prior conversation compacted]',
      }
    }
    if (resp?.status === 'busy') return { status: 'busy' }
    if (resp?.status === 'empty') return { status: 'empty' }
    if (resp?.status === 'overflow') {
      return { status: 'overflow', message: 'compaction unavailable' }
    }
    return {
      status: 'error',
      message: `unexpected status: ${String(
        (resp as { status?: unknown })?.status ?? 'null',
      )}`,
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
