import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { isWorkerFunction, unwrapEnvelope } from './parsers'
import { WorkerListView } from './WorkerListView'
import {
  WorkerAddView,
  WorkerClearView,
  WorkerRemoveView,
  WorkerStartView,
  WorkerStopView,
  WorkerUpdateView,
} from './WorkerOpView'

export function WorkerFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('worker::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('worker::'.length)
  return (
    <>
      <span className="text-ink-faint">worker::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isWorkerFunction(message.functionId)) return null
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
    case 'worker::list':
      return <WorkerListView input={input} output={output} running={running} />
    case 'worker::start':
      return <WorkerStartView input={input} output={output} running={running} />
    case 'worker::stop':
      return <WorkerStopView input={input} output={output} running={running} />
    case 'worker::add':
      return <WorkerAddView input={input} output={output} running={running} />
    case 'worker::remove':
      return (
        <WorkerRemoveView input={input} output={output} running={running} />
      )
    case 'worker::update':
      return (
        <WorkerUpdateView input={input} output={output} running={running} />
      )
    case 'worker::clear':
      return <WorkerClearView input={input} output={output} running={running} />
    case 'worker::schema':
      // Schema introspection — wire shape is opaque per-call, defer to
      // default JSON pane until we have a concrete UX.
      return null
    default:
      return null
  }
}

/**
 * Destructive lifecycle ops (start/stop/add/remove/update/clear) are
 * approval-gated by the daemon, but the pending-state payload echoes the
 * caller request — so the default request JSON pane is already a fine
 * preview. Returning `null` keeps the implementation focused on the
 * done/running paths.
 */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const WorkerToolView = {
  isWorkerFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
