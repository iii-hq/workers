import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { FunctionInfoView } from './FunctionInfoView'
import { FunctionsListView } from './FunctionsListView'
import { ObserverInfoView } from './ObserverInfoView'
import { ObserversListView } from './ObserversListView'
import { ObserverAddedDisplay, ObserveView } from './ObserveView'
import {
  coerceJsonObject,
  isEngineListFunction,
  OBSERVE_FUNCTION_ID,
  parseFunctionInfoResponse,
  parseObserverInfoResponse,
  unwrapEnvelope,
} from './parsers'
import { UnobserveView } from './UnobserveView'
import { WorkerInfoView } from './WorkerInfoView'
import { WorkersListView } from './WorkersListView'
import { WorkersRegisterView } from './WorkersRegisterView'

/**
 * Header label for `engine::*` ids. Mirrors the other families'
 * FunctionIdLabels: dims the namespace prefix so the tail is readable in
 * the FCM header.
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

  // Coerce a double-encoded (stringified-JSON) payload back to an object so
  // the structured views parse it, instead of showing an escaped one-liner.
  const input = coerceJsonObject(unwrapEnvelope(message.input))
  const rawOutput = message.output
  const output =
    rawOutput != null ? coerceJsonObject(unwrapEnvelope(rawOutput)) : undefined
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
    case 'engine::functions::info': {
      if (running) return <FunctionInfoView input={input} running />
      // Parse HERE, not in the view: returning null makes the card fall back
      // to the generic request/response panes — a view that matched but
      // renders nothing would leave a blank terminal tab.
      const details = parseFunctionInfoResponse(rawOutput)
      if (!details) return null
      return <FunctionInfoView input={input} details={details} />
    }
    case 'engine::observers::list':
      return (
        <ObserversListView input={input} output={output} running={running} />
      )
    case 'engine::observers::get': {
      if (running) return <ObserverInfoView input={input} running />
      // Same contract as functions::info: unparseable settled output returns
      // null so the card falls back to the generic panes, never a blank tab.
      const observer = parseObserverInfoResponse(rawOutput)
      if (!observer) return null
      return <ObserverInfoView input={input} observer={observer} />
    }
    case 'engine::observers::dispose':
    case 'harness::unobserve':
      return <UnobserveView input={input} output={output} running={running} />
    case 'engine::workers::list':
      return <WorkersListView input={input} output={output} running={running} />
    case 'engine::workers::info':
      return <WorkerInfoView input={input} output={output} running={running} />
    case 'engine::workers::register':
      return (
        <WorkersRegisterView input={input} output={output} running={running} />
      )
    case OBSERVE_FUNCTION_ID:
      return (
        <ObserveView
          messageId={message.id}
          input={input}
          output={output}
          running={running}
        />
      )
    default:
      return null
  }
}

/**
 * Engine reads don't go through the approval gate, so there's nothing
 * meaningful to preview. Returning `null` lets the
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

function tryRenderObserveDisplay(
  message: FunctionCallMessage,
): React.ReactNode | null {
  if (
    message.functionId !== OBSERVE_FUNCTION_ID ||
    message.pendingApproval ||
    message.running ||
    parseSandboxErrorDisplay(message.output)
  ) {
    return null
  }
  return (
    <ObserverAddedDisplay
      messageId={message.id}
      input={coerceJsonObject(unwrapEnvelope(message.input))}
      output={
        message.output != null
          ? coerceJsonObject(unwrapEnvelope(message.output))
          : undefined
      }
      createdAt={message.createdAt}
    />
  )
}

const isObserveFunction = (functionId: string) =>
  functionId === OBSERVE_FUNCTION_ID

function tryRenderObserve(
  message: FunctionCallMessage,
): React.ReactNode | null {
  if (!isObserveFunction(message.functionId)) return null
  return tryRender(message)
}

/** Focused entry registered ahead of the engine family so an agent's observe
 * gets a compact prominent receipt without promoting every engine function. */
export const ObserveToolView = {
  isObserveFunction,
  tryRender: tryRenderObserve,
  tryRenderRunning: tryRenderObserve,
  tryRenderDisplay: tryRenderObserveDisplay,
}
