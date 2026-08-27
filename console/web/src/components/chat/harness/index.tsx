import type { FunctionTriggerMessage } from '@/types/chat'
import { isHarnessFunction } from './parsers'
import { SubmitResultView } from './SubmitResultView'

/**
 * Harness tool family — the one synthetic tool the harness injects into a
 * turn (`submit_result`, the output-contract fallback). `harness::spawn`
 * still carries the id (so its label still brands `harness::`), but renders
 * through the plain default JSON view like any other function call: no
 * special card, no live child-session surface. A caller who wants the
 * result back binds a trigger, same as everything else on the bus.
 */
export { HARNESS_FUNCTION_IDS, isHarnessFunction } from './parsers'

/** Branded function-id label, mirroring the other namespace modules. */
export function HarnessFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('harness::')) {
    return <span className="text-ink font-medium">{functionId}</span>
  }
  const tail = functionId.slice('harness::'.length)
  return (
    <>
      <span className="text-ink-faint">harness::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isHarnessFunction(message.functionId)) return null
  if (message.pendingApproval) return null
  if (message.functionId !== 'submit_result') return null
  return (
    <SubmitResultView input={message.input} running={!!message.running} />
  )
}

export const HarnessToolView = {
  isHarnessFunction,
  tryRender,
  /** Running state is handled inside the views. */
  tryRenderRunning: tryRender,
}
