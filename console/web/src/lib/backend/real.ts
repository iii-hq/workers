/**
 * iii-browser-sdk + harness turn kickoff. Deployment permission rules live in
 * the `approval-gate` configuration entry; the console derives the harness
 * structural floor from those rules on each send. Operating mode is passed to
 * the harness, which assembles the provider-specific identity prompt when
 * `system_prompt` is omitted.
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
import { loadApprovalGateDefaults } from './approval-gate-config'
import {
  getTurnStatus,
  type HarnessFunctionPolicy,
  type HarnessSendRequest,
  type HarnessThinkingLevel,
  isTurnActive,
  sendTurn,
  stopTurn,
} from './harness-send'
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
  QueuedMessagePreview,
  StreamEvent,
} from './types'

interface RunParams {
  provider: string
  model: string
}

/**
 * The chat composer is a general-purpose agent surface: expose the whole bus
 * via `agent_trigger`. The approval-gate rules supply the structural floor;
 * the gate hook remains the human decision surface.
 */
export const FALLBACK_FUNCTION_POLICY: HarnessFunctionPolicy = {
  allow: ['*'],
  deny: ['approval::*', 'configuration::*', 'shell::workspace::*'],
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

/** Build `harness::send` `options.metadata` for filesystem scope. */
export function buildTurnMetadata(
  sessionId: string,
  messageId: string,
  workingDir?: string | null,
): Record<string, unknown> {
  return {
    session_id: sessionId,
    message_id: messageId,
    ...(workingDir ? { fs_scope: { root: workingDir } } : {}),
  }
}

/**
 * Assemble the `harness::send` request shared by the stream kickoff and the
 * mid-stream queue path — model/provider resolution, thinking level, the
 * approval-gate structural floor, and the string-sugar vs structured message
 * form.
 */
async function buildSendRequest(
  prompt: string,
  mode: Mode,
  model: ModelId,
  sessionId: string,
  messageId: string,
  opts?: ChatStreamOptions,
): Promise<HarnessSendRequest> {
  const { provider, model: modelId } = resolveRunParams(model)
  const thinkingLevel = toThinkingLevel(opts?.thinkingLevel)

  let functionPolicy = FALLBACK_FUNCTION_POLICY
  try {
    functionPolicy = (await loadApprovalGateDefaults()).functionPolicy
  } catch (err) {
    if (import.meta.env.DEV) {
      console.warn(
        '[real-backend] approval-gate config unavailable; using fallback policy',
        err,
      )
    }
  }

  // Attachment blocks (file-mention expansions) upgrade the message to the
  // structured MessageInput form; plain sends keep the string sugar so the
  // wire payload stays byte-identical for the common case.
  const attachedBlocks = opts?.attachedBlocks ?? []
  const message =
    attachedBlocks.length > 0
      ? {
          role: 'user' as const,
          content: [prompt, ...attachedBlocks].map((text) => ({
            type: 'text' as const,
            text,
          })),
          timestamp: Date.now(),
        }
      : prompt

  return {
    session_id: sessionId,
    message,
    model: modelId,
    provider,
    idempotency_key: messageId,
    session: { metadata: { surface: 'console' } },
    options: {
      mode,
      functions: functionPolicy,
      ...(thinkingLevel ? { thinking_level: thinkingLevel } : {}),
      metadata: buildTurnMetadata(sessionId, messageId, opts?.workingDir),
    },
  }
}

/**
 * Mid-stream send (MOT-3837): the turn is already streaming, so just trigger
 * `harness::send` — the harness queues the message and delivers it after the
 * stream ends. No second stream loop: the live one keeps rendering, and the
 * queued row lands via session events on drain.
 */
async function realQueueMessage(
  prompt: string,
  mode: Mode,
  model: ModelId,
  opts?: ChatStreamOptions,
): Promise<void> {
  const client = await getIiiClient()
  const sessionId = opts?.sessionId ?? `console-${crypto.randomUUID()}`
  const messageId = opts?.messageId ?? newMessageId()
  const req = await buildSendRequest(
    prompt,
    mode,
    model,
    sessionId,
    messageId,
    opts,
  )
  await sendTurn(client, req)
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
  // The `approval::*` functions live on the optional standalone approval-gate
  // worker. When it's absent, skip every approval subscription / read so we
  // don't register triggers for unknown types or call missing functions.
  const approvalGateAvailable = opts?.approvalGateAvailable !== false

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
  const stopApprovalEvents = approvalGateAvailable
    ? startApprovalEventsSubscription(client, sessionId, {
        onCreated: (record) =>
          pushPending(record.function_call_id, {
            kind: 'approval-created',
            record,
          }),
        onResolved: (event) => push({ kind: 'approval-resolved', event }),
      })
    : () => {}

  const onAbort = () => wake()
  signal?.addEventListener('abort', onAbort, { once: true })

  let kickoffError: Error | null = null
  try {
    // Reconnect recovery: rebuild any pending-approval cards a prior turn left
    // held (e.g. resending after a reload). Best-effort; skipped without the gate.
    if (approvalGateAvailable) {
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
    }

    // Kick off (or steer) the turn. A turn already running for this session
    // folds the message in (merge/queue) rather than rejecting — no busy error.
    const sendRequest = await buildSendRequest(
      prompt,
      mode,
      model,
      sessionId,
      messageId,
      opts,
    )
    void sendTurn(client, sendRequest).catch((err) => {
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

/**
 * The session's server-side message queue: `harness::status` → `queued` rows
 * mapped to previews keyed by their eventual transcript entry id. Includes
 * messages queued by other tabs and subagent/subscription notifications.
 */
async function realListQueued(
  sessionId: string,
): Promise<QueuedMessagePreview[]> {
  const client = await getIiiClient()
  const status = await getTurnStatus(client, sessionId).catch(() => null)
  return (status?.queued ?? []).map((row) => ({
    id: row.entry_id,
    text: (row.message.content ?? [])
      .filter((b) => b.type === 'text' && typeof b.text === 'string')
      .map((b) => b.text)
      .join('\n'),
    queuedAt: row.queued_at,
  }))
}

async function realResolveApproval(
  sessionId: string,
  functionCallId: string,
  decision: 'allow' | 'deny',
  opts?: { accessDuration?: 'once' | 'session' | 'always' },
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('approval::resolve', {
    session_id: sessionId,
    function_call_id: functionCallId,
    decision,
    // Only ride `access_duration` on filesystem_access resolutions.
    ...(opts?.accessDuration ? { access_duration: opts.accessDuration } : {}),
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
  queueMessage: realQueueMessage,
  listQueued: realListQueued,
  resolveApproval: realResolveApproval,
  abortRun: realAbortRun,
  compactSession: realCompactSession,
}
