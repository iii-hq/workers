import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { FunctionInfoView } from './FunctionInfoView'
import { FunctionsListView } from './FunctionsListView'
import { isEngineListFunction, unwrapEnvelope } from './parsers'
import { RegisteredTriggersListView } from './RegisteredTriggersListView'
import { RegisterTriggerView } from './RegisterTriggerView'
import { TriggersListView } from './TriggersListView'
import { WorkerInfoView } from './WorkerInfoView'
import { WorkersListView } from './WorkersListView'
import { WorkersRegisterView } from './WorkersRegisterView'

/**
 * Header label for `engine::*::list` ids. Mirrors `SandboxFunctionIdLabel`:
 * dims the namespace prefix so the tail is readable in the FCM header.
 */
export function EngineFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('engine::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('engine::'.length)
  return (
    <>
      <span className="text-ink-faint">engine::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isEngineListFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  // Reuse the sandbox error parser for gate/transport-level errors
  // — the `function_error` envelope is shared infra, not sandbox-specific.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'engine::functions::list':
      return (
        <FunctionsListView input={input} output={output} running={running} />
      )
    case 'engine::functions::info':
      return (
        <FunctionInfoView input={input} output={output} running={running} />
      )
    case 'engine::triggers::list':
      return (
        <TriggersListView input={input} output={output} running={running} />
      )
    case 'engine::registered-triggers::list':
      return (
        <RegisteredTriggersListView
          input={input}
          output={output}
          running={running}
        />
      )
    case 'engine::workers::list':
      return <WorkersListView input={input} output={output} running={running} />
    case 'engine::workers::info':
      return <WorkerInfoView input={input} output={output} running={running} />
    case 'engine::workers::register':
      return (
        <WorkersRegisterView input={input} output={output} running={running} />
      )
    case 'engine::register_trigger':
      return (
        <RegisterTriggerView input={input} output={output} running={running} />
      )
    default:
      return null
  }
}

/**
 * Engine list calls are read-only and don't go through the approval gate,
 * so there's nothing meaningful to preview. Returning `null` lets the
 * default request JSON pane render if a `pendingApproval` message ever
 * reaches the renderer.
 */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const EngineToolView = {
  isEngineListFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
