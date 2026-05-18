/**
 * Real backend: iii-browser-sdk + harness fanout.
 *
 * Each `stream()` call:
 *   1. Reads `opts.sessionId` (stable across all turns of a chat
 *      conversation) and mints a fresh `message_id` (one per send).
 *      Mirrors `workers/harness/web/src/App.tsx` `send()` — the same
 *      session_id is reused for every turn, the message_id is new per
 *      user message. Both flow into the engine via baggage so the
 *      traces UI can group spans by session AND by message.
 *      If `opts.sessionId` is missing (legacy callers), falls back to
 *      a fresh `console-<uuid>` so behavior is at least well-defined.
 *   2. Registers a per-`ui::session::event::<browser_id>` handler that
 *      enqueues envelopes whose `session_id` matches.
 *   3. Calls `ui::subscribe { browser_id, session_id }` so the harness
 *      fanout starts forwarding this session's events to us.
 *   4. Fires `run::start { session_id, message_id, provider, model,
 *      mode, messages }` against turn-orchestrator. The harness owns
 *      the system prompt; the console only ships the mode and lets the
 *      harness prepend the matching mode paragraph to its identity
 *      preamble.
 *   5. Pumps the queue through `translateAgentEvent`, yielding each
 *      resulting `StreamEvent`. Terminates on the `agent_end` envelope.
 *
 * Honors `opts.signal`: if the caller aborts, the generator returns and
 * the `finally` runs `ui::unsubscribe` and unregisters the handler.
 *
 * Phase 2.B (§D): the per-call `approval_required` array no longer
 * exists. Permissions are owned by the harness's `iii-permissions.yaml`
 * (loaded from the harness cwd; watched for changes). The chat surface
 * just describes the desired mode — the operator owns policy.
 *
 * Remaining caveats (each is a deliberate Phase 2/3/4 target — see
 * `PHASE-2-PLAN.md`):
 *   - Assistant body arrives as a single `assistant-token` (no per-token
 *     streaming yet; Phase 2.A wires `message_update`).
 *   - Provider/model selection uses the models-catalog via the picker; legacy
 *     ids without `::` still get a coarse provider guess in `resolveRunParams`.
 */

import { parseCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import { newMessageId } from '@/lib/session-id'
import type { Mode, ModelId } from '@/types/chat'
import type { AgentEvent, SessionEventEnvelope } from '@/types/iii-agent-event'
import { translateAgentEvent } from './translate'
import type { ChatBackend, ChatStreamOptions, StreamEvent } from './types'

interface RunParams {
  provider: string
  model: string
}

/**
 * Map console model selection onto turn-orchestrator's `provider` / `model`
 * fields. The system prompt is no longer the console's concern — the
 * harness owns it and is fed `mode` directly on `run::start`.
 *
 * Model ids from the catalog picker are `provider::<catalog_id>`. Legacy
 * heuristic ids (no `::`) still map `claude*` / `gemini*` / default openai.
 */
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
  // Prefer the caller-supplied conversation-scoped session_id; fall back
  // to a fresh per-call id only when the caller hasn't been updated yet.
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

  // Wake any pending await when the caller aborts so the loop can exit.
  const onAbort = () => wake()
  signal?.addEventListener('abort', onAbort, { once: true })

  let subscribed = false

  try {
    await client.call('ui::subscribe', {
      browser_id: client.browserId,
      session_id: sessionId,
    })
    subscribed = true

    const { provider, model: modelId } = resolveRunParams(model)

    // Route the turn-orchestrator kick-off through `harness::call` so the
    // harness wraps the inner trigger with an instrumented span that
    // seeds `iii.session.id` / `iii.message.id` as baggage. Without this
    // wrapper, the engine's `engine::traces::group_by` returns empty
    // results for "Group by session" / "Group by message" because the
    // engine's own `handle_invocation` / `call run::start` spans aren't
    // inside any baggage context. Mirrors `workers/harness/web/src/App.tsx`
    // `send()` (it uses the HTTP bridge — we use the WS bus). Errors are
    // non-fatal at the contract level (the UI surfaces them through the
    // assistant stream); log and let the loop fall through.
    void client
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
            {
              role: 'user',
              content: [{ type: 'text', text: prompt }],
              timestamp: Date.now(),
            },
          ],
        },
      })
      .catch((err) => {
        console.warn('[real-backend] harness::call run::start failed', err)
      })

    while (true) {
      if (signal?.aborted) return
      while (queue.length === 0) {
        if (signal?.aborted) return
        await new Promise<void>((resolve) => {
          resolveNext = resolve
        })
      }
      const event = queue.shift()
      if (!event) continue
      for (const streamEvent of translateAgentEvent(event, sessionId)) {
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

export const realBackend: ChatBackend = {
  id: 'real',
  stream: realStream,
  resolveApproval: realResolveApproval,
}
