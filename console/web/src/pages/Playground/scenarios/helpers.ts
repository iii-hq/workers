import type { ChatBackend, ChatStreamOptions, StreamEvent } from '@/lib/backend'
import type { Mode, ModelId } from '@/types/chat'

export type ScenarioStream = (
  prompt: string,
  mode: Mode,
  model: ModelId,
  opts?: ChatStreamOptions,
) => AsyncGenerator<StreamEvent>

export function makeBackend(id: string, stream: ScenarioStream): ChatBackend {
  return { id, stream }
}

export function tokenize(body: string): string[] {
  return body.split(/(\s+)/)
}

export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) return resolve()
    const t = setTimeout(resolve, ms)
    signal?.addEventListener(
      'abort',
      () => {
        clearTimeout(t)
        resolve()
      },
      { once: true },
    )
  })
}

interface ThoughtOptions {
  signal?: AbortSignal
  /** mean delay between thought tokens, in ms */
  meanDelayMs?: number
  /** if set, hard-stop after this fraction of tokens (0..1] */
  truncateAt?: number
}

export async function* streamThought(
  body: string,
  options: ThoughtOptions = {},
): AsyncGenerator<StreamEvent> {
  const { signal, meanDelayMs = 14, truncateAt } = options
  const startedAt = Date.now()
  yield { kind: 'thought-start' }
  const tokens = tokenize(body)
  const limit =
    typeof truncateAt === 'number'
      ? Math.max(1, Math.floor(tokens.length * truncateAt))
      : tokens.length
  for (let i = 0; i < limit; i++) {
    if (signal?.aborted) return
    const tok = tokens[i]
    if (tok) yield { kind: 'thought-token', token: tok }
    await sleep(meanDelayMs * (0.6 + Math.random() * 0.8), signal)
  }
  if (typeof truncateAt === 'number') return
  yield { kind: 'thought-end', durationMs: Date.now() - startedAt }
}

interface AssistantOptions {
  signal?: AbortSignal
  meanDelayMs?: number
  /** when false, do NOT yield assistant-end (used by truncation tests) */
  emitEnd?: boolean
}

export async function* streamAssistant(
  body: string,
  options: AssistantOptions = {},
): AsyncGenerator<StreamEvent> {
  const { signal, meanDelayMs = 22, emitEnd = true } = options
  for (const tok of tokenize(body)) {
    if (signal?.aborted) return
    if (tok) yield { kind: 'assistant-token', token: tok }
    await sleep(meanDelayMs * (0.5 + Math.random()), signal)
  }
  if (emitEnd) yield { kind: 'assistant-end' }
}

interface FcallOptions {
  functionId: string
  input: unknown
  output: unknown
  /** how long the call "runs" before fcall-end */
  waitMs?: number
  /** when true, emit pendingApproval and wait for an arbitrary timeout */
  pendingApproval?: boolean
  /** signal */
  signal?: AbortSignal
  /** how long to "wait for approval" before resolving */
  approvalWaitMs?: number
}

export async function* streamFcall(
  options: FcallOptions,
): AsyncGenerator<StreamEvent> {
  const {
    functionId,
    input,
    output,
    waitMs = 600,
    pendingApproval,
    signal,
    approvalWaitMs = 1500,
  } = options
  const startedAt = Date.now()
  yield {
    kind: 'fcall-start',
    functionId,
    input,
    pendingApproval,
  }
  if (pendingApproval) {
    /* simulate the lag between "user clicks approve" and the call running */
    await sleep(approvalWaitMs, signal)
    if (signal?.aborted) return
  }
  await sleep(waitMs, signal)
  if (signal?.aborted) return
  yield { kind: 'fcall-end', output, durationMs: Date.now() - startedAt }
}
