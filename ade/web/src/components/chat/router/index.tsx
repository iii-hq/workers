import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionTriggerMessage } from '@/types/chat'
import { ModelsListView } from './ModelsListView'
import { canonicalRouterId, isRouterFunction, unwrapEnvelope } from './parsers'

/* The known llm-router::* set lives in parsers.ts (ROUTER_FUNCTION_IDS) so the
   dispatcher and schemas share one source of truth. */
export { isRouterFunction, ROUTER_FUNCTION_IDS } from './parsers'

/** Branded function-id label, mirroring the other namespace modules. */
export function RouterFunctionIdLabel({ functionId }: { functionId: string }) {
  const prefix = ['llm-router::', 'router::'].find((p) =>
    functionId.startsWith(p),
  )
  if (!prefix) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice(prefix.length)
  return (
    <>
      <span className="text-ink-faint">{prefix}</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isRouterFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  // Reuse the sandbox error parser for gate/transport-level errors — the
  // `function_error` envelope is shared infra, not sandbox-specific.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (canonicalRouterId(message.functionId)) {
    case 'llm-router::models::list':
      return <ModelsListView input={input} output={output} running={running} />
    default:
      return null
  }
}

/**
 * Router list calls are read-only and don't go through the approval gate, so
 * there's nothing meaningful to preview.
 */
function tryRenderPreview(
  _message: FunctionTriggerMessage,
): React.ReactNode | null {
  return null
}

export const RouterToolView = {
  isRouterFunction,
  tryRender,
  /** Alias kept for FCM symmetry; running state lives inside `tryRender`. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
