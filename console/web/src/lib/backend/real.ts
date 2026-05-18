/**
 * Real backend: iii-browser-sdk + harness fanout.
 *
 * Each `stream()` call:
 *   1. Mints a fresh `session_id` (Phase 1 is one-shot; multi-turn session
 *      persistence is Phase 4).
 *   2. Registers a per-`ui::session::event::<browser_id>` handler that
 *      enqueues envelopes whose `session_id` matches.
 *   3. Calls `ui::subscribe { browser_id, session_id }` so the harness
 *      fanout starts forwarding this session's events to us.
 *   4. Fires `run::start { session_id, provider, model, mode, messages }`
 *      against turn-orchestrator. The harness owns the system prompt; the
 *      console only ships the mode and lets the harness prepend the
 *      matching mode paragraph to its identity preamble.
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
 *   - Approvals surface as `pendingApproval: true` but the UI can't yet
 *     resolve them (Phase 3 adds approve/deny buttons calling
 *     `approval::resolve`).
 *   - Provider/model selection uses the models-catalog via the picker; legacy
 *     ids without `::` still get a coarse provider guess in `resolveRunParams`.
 */

import { getIiiClient } from '@/lib/iii-client'
import { parseCatalogModelKey } from '@/lib/catalog-model-key'
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
  const sessionId = `console-${crypto.randomUUID()}`

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

    // Fire-and-forget: run::start kicks off the turn-orchestrator state
    // machine; events arrive via the ui::session::event handler above.
    // Errors here are non-fatal at the contract level (the UI surfaces
    // them via the assistant stream); log and let the loop fall through.
    void client
      .call('run::start', {
        session_id: sessionId,
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
      })
      .catch((err) => {
        console.warn('[real-backend] run::start failed', err)
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
      for (const streamEvent of translateAgentEvent(event)) {
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

export const realBackend: ChatBackend = {
  id: 'real',
  stream: realStream,
}
