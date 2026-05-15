import type { Mode, ModelId } from '@/types/chat'
import type { ChatBackend, ChatStreamOptions, StreamEvent } from './types'

/**
 * Placeholder for the eventual real backend. Wire your provider here while
 * preserving the StreamEvent contract documented in PLAYGROUND.md, and the
 * playground will exercise the new implementation without changes.
 */
// biome-ignore lint/correctness/useYield: stub backend; throws before yielding.
async function* realStream(
  _prompt: string,
  _mode: Mode,
  _model: ModelId,
  _opts?: ChatStreamOptions,
): AsyncGenerator<StreamEvent> {
  throw new Error(
    'backend not configured — see chat-app/PLAYGROUND.md for the streaming contract',
  )
}

export const realBackend: ChatBackend = {
  id: 'real',
  stream: realStream,
}
