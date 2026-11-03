import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { isStateFunction } from './parsers'
import { StateView } from './StateView'

/**
 * Header label for `state::*` ids — dims the namespace prefix so the op
 * (`get`, `set`, …) reads clearly. Mirrors `EngineFunctionIdLabel`.
 */
export function StateFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('state::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('state::'.length)
  return (
    <>
      <span className="text-ink-faint">state::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isStateFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const running = !!message.running
  const rawOutput = message.output

  // Reuse the shared error parser for gate/transport-level errors — the
  // `function_error` envelope is shared infra, not sandbox-specific. But state
  // values are arbitrary JSON, and a *successful* result whose stored value
  // looks like a denial (e.g. `{ status: 'denied' }`, `{ denied_by: … }`) would
  // otherwise be misread as an error. A real error is `{ error: { kind:
  // 'function_error', … } }`; a success is a `{ content, details }` harness
  // envelope. Skip the parser for success envelopes so only genuine errors —
  // which are never `{ content, details }` shaped — reach it.
  const isSuccessEnvelope =
    !!rawOutput &&
    typeof rawOutput === 'object' &&
    !Array.isArray(rawOutput) &&
    Array.isArray((rawOutput as Record<string, unknown>).content) &&
    'details' in (rawOutput as Record<string, unknown>)
  const errorDisplay =
    !running && rawOutput != null && !isSuccessEnvelope
      ? parseSandboxErrorDisplay(rawOutput)
      : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  return (
    <StateView
      functionId={message.functionId}
      input={message.input}
      output={rawOutput}
      running={running}
    />
  )
}

/** `state::*` calls have no bespoke pending preview. */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const StateToolView = {
  isStateFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
