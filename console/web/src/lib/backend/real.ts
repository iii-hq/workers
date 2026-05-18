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

// Legacy heuristic ids (no `::`) map `claude*` → anthropic, `gemini*` →
// google, everything else → openai.
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

    // Fire-and-forget: events flow back via ui::session::event. Errors
    // here are surfaced through the assistant stream, not the caller.
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
