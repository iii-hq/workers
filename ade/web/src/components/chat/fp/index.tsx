import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionTriggerMessage } from '@/types/chat'
import { PipeView } from './PipeView'
import {
  FP_PIPE_ID,
  FP_PREFIX,
  isFpFunction,
  isFpTransformFunction,
  unwrapEnvelope,
} from './parsers'
import { UtilView } from './UtilView'

/**
 * fp tool family — the standalone fp worker: `fp::pipe` (the
 * worker-side pipeline) and the ten pure `fp::*` transforms
 * (get/pick/omit/take/drop/map/filter/split/join/uniq).
 */
export { isFpFunction } from './parsers'

/** Route genuine `{ error: { kind: 'function_error' } }` envelopes to the
    shared error view. Pipe/transform results carry arbitrary threaded JSON,
    so skip the parser for `{ content, details }` success envelopes — a value
    that happens to look like a denial must not be misread (mirrors state). */
function guardedErrorDisplay(rawOutput: unknown, running: boolean) {
  if (running || rawOutput == null) return null
  const isSuccessEnvelope =
    typeof rawOutput === 'object' &&
    !Array.isArray(rawOutput) &&
    Array.isArray((rawOutput as Record<string, unknown>).content) &&
    'details' in (rawOutput as Record<string, unknown>)
  if (isSuccessEnvelope) return null
  return parseSandboxErrorDisplay(rawOutput)
}

/** Branded function-id label, mirroring the other namespace modules. */
export function FpFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith(FP_PREFIX)) {
    return <span className="text-ink font-medium">{functionId}</span>
  }
  const tail = functionId.slice(FP_PREFIX.length)
  return (
    <>
      <span className="text-ink-faint">{FP_PREFIX}</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isFpFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const running = !!message.running
  const rawOutput = message.output
  const errorDisplay = guardedErrorDisplay(rawOutput, running)
  if (errorDisplay) return <SandboxErrorView display={errorDisplay} />

  if (message.functionId === FP_PIPE_ID) {
    return (
      <PipeView
        input={message.input}
        // the ONE unwrap — PipeView must never unwrap again
        output={rawOutput != null ? unwrapEnvelope(rawOutput) : undefined}
        running={running}
      />
    )
  }
  if (!isFpTransformFunction(message.functionId)) return null
  return (
    <UtilView
      functionId={message.functionId}
      input={message.input}
      output={rawOutput}
      running={running}
    />
  )
}

/** The pipe is approval-gated by default (its steps run with worker
    authority), so the step list IS what an approver reviews — render it as
    the preview. */
function tryRenderPreview(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (message.functionId !== FP_PIPE_ID) return null
  return <PipeView input={message.input} />
}

export const FpToolView = {
  isFpFunction,
  tryRender,
  /** Running state is handled inside the views. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
