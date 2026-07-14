import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import {
  ActView,
  ScreenshotView,
  SessionListView,
  SessionStartView,
  SessionStopView,
} from './ComputerViews'
import { isComputerFunction } from './parsers'

/**
 * Header label for `computer::*` ids: dims the namespace prefix so the op
 * (`screenshot`, `act`, `sessions::start`, …) reads clearly. Mirrors
 * `BrowserFunctionIdLabel`.
 */
export function ComputerFunctionIdLabel({
  functionId,
}: {
  functionId: string
}) {
  if (!functionId.startsWith('computer::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('computer::'.length)
  return (
    <>
      <span className="text-ink-faint">computer::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isComputerFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const running = !!message.running
  const rawOutput = message.output

  // Shared infra errors (gate denials, dispatch policy, function_error
  // envelopes). Computer success payloads never look denial-shaped: results are
  // `ok`-flagged structs or `{content, details}` screenshot envelopes.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  const fid = message.functionId

  // Screenshot / observe read the image blocks straight off the raw output —
  // `unwrapEnvelope` would drop them by returning `details` — and carry their
  // own capturing state.
  if (fid === 'computer::screenshot' || fid === 'computer::observe') {
    return (
      <ScreenshotView
        functionId={fid}
        input={message.input}
        output={rawOutput}
        running={running}
      />
    )
  }

  // The flat ops resolve near-instantly; while running they fall back to the
  // card's default response pane.
  if (running) return null

  switch (fid) {
    case 'computer::sessions::start':
      return <SessionStartView output={rawOutput} />
    case 'computer::sessions::list':
      return <SessionListView output={rawOutput} />
    case 'computer::sessions::stop':
      return <SessionStopView output={rawOutput} />
    case 'computer::act':
      return <ActView input={message.input} output={rawOutput} />
    default:
      // Internal screencast / frame plumbing — no agent-facing view; fall back
      // to the card's default request/response JSON panes.
      return null
  }
}

/** `computer::*` calls have no bespoke pending preview. */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const ComputerToolView = {
  isComputerFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
