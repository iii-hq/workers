import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { NodeResultView } from './NodeResultView'
import { isWorkflowFunction, unwrapEnvelope } from './parsers'
import { StartView } from './StartView'
import { StatusView } from './StatusView'
import { StopView } from './StopView'

/* The known workflow::* set lives in parsers.ts (WORKFLOW_FUNCTION_IDS) so the
   dispatcher and schemas share one source of truth. */
export { isWorkflowFunction, WORKFLOW_FUNCTION_IDS } from './parsers'

/** Branded function-id label, mirroring the other namespace modules. */
export function WorkflowFunctionIdLabel({
  functionId,
}: {
  functionId: string
}) {
  if (!functionId.startsWith('workflow::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('workflow::'.length)
  return (
    <>
      <span className="text-ink-faint">workflow::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isWorkflowFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'workflow::start':
      return <StartView input={input} output={output} running={running} />
    case 'workflow::status':
      return <StatusView input={input} output={output} running={running} />
    case 'workflow::node-result':
      return <NodeResultView input={input} output={output} running={running} />
    case 'workflow::stop':
      return <StopView input={input} output={output} running={running} />
    default:
      // Internal ids (tick/sweep/wake) fall back to the default JSON pane.
      return null
  }
}

/**
 * No bespoke pending preview: the request JSON pane already shows the
 * definition before approval, and approvals are rare on workflow calls.
 */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const WorkflowToolView = {
  isWorkflowFunction,
  tryRender,
  /** Alias kept for FCM symmetry; running state lives inside `tryRender`. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
