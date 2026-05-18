import type { Mode, ModelId } from '@/types/chat'

export type StreamEvent =
  | { kind: 'thought-start' }
  | { kind: 'thought-token'; token: string }
  | { kind: 'thought-end'; durationMs: number }
  | {
      kind: 'fcall-start'
      functionId: string
      input: unknown
      pendingApproval?: boolean
      functionCallId?: string
      sessionId?: string
    }
  | { kind: 'fcall-end'; output: unknown; durationMs: number }
  | { kind: 'assistant-token'; token: string }
  | { kind: 'assistant-end' }

export interface ChatStreamOptions {
  signal?: AbortSignal
  meanDelayMs?: number
}

export interface ChatBackend {
  readonly id: string
  stream(
    prompt: string,
    mode: Mode,
    model: ModelId,
    opts?: ChatStreamOptions,
  ): AsyncGenerator<StreamEvent>
  resolveApproval?(
    sessionId: string,
    functionCallId: string,
    decision: 'allow' | 'deny',
  ): Promise<void>
}
