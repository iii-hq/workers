import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { FetchPreview, FetchView } from './FetchView'
import {
  fetchRequestSchema,
  isWebFunction,
  safeParseRequest,
  unwrapEnvelope,
} from './parsers'

export function WebFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('web::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('web::'.length)
  return (
    <>
      <span className="text-ink-faint">web::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isWebFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  // Reuse the sandbox error parser for transport-level / gate denials.
  // `web::fetch`'s own handler-level errors (`{ ok: false, ... }`) are
  // handled inside FetchView's success/error union, not here.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'web::fetch':
      return <FetchView input={input} output={output} running={running} />
    default:
      return null
  }
}

function tryRenderPreview(
  message: FunctionCallMessage,
): React.ReactNode | null {
  if (!isWebFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  // Parse-check BEFORE building an element: FetchPreview renders null for
  // an unparseable request, but a non-null element here makes
  // FunctionCallCard suppress its raw request pane (the null contract).
  switch (message.functionId) {
    case 'web::fetch':
      return safeParseRequest(fetchRequestSchema, input) ? (
        <FetchPreview input={input} />
      ) : null
    default:
      return null
  }
}

export const WebToolView = {
  isWebFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
