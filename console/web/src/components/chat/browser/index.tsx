import { SquareArrowOutUpRight } from 'lucide-react'
import { hashForBrowserSession } from '@/hooks/use-hash-route'
import {
  browserSessionIdFromCall,
  isBrowserFunction,
  parseScreenshotOutput,
} from '@/lib/browser'
import { JsonHighlight } from '@/lib/syntax'
import type { FunctionCallMessage } from '@/types/chat'

/**
 * Header label for `browser::*` ids: dims the namespace prefix so the op
 * (`navigate`, `act`, …) reads clearly. Mirrors `StateFunctionIdLabel`.
 */
export function BrowserFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('browser::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('browser::'.length)
  return (
    <>
      <span className="text-ink-faint">browser::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

/**
 * Terminal view for a `browser::*` call: the owning session with an
 * "open in browser tab" affordance (routes to `#/browser/<session_id>`),
 * plus the captured image for screenshots or the result JSON otherwise.
 * The raw json tab keeps the untouched request/response panes.
 */
function BrowserCallView({ message }: { message: FunctionCallMessage }) {
  const sessionId = browserSessionIdFromCall(message.input, message.output)
  const screenshot =
    message.functionId === 'browser::screenshot' && message.output != null
      ? parseScreenshotOutput(message.output)
      : null
  const running = !!message.running

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2 px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[11px] lowercase">
        {sessionId ? (
          <span className="text-ink-faint min-w-0 truncate">
            session <span className="text-ink tabular-nums">{sessionId}</span>
          </span>
        ) : (
          <span className="text-ink-faint">browser</span>
        )}
        {sessionId ? (
          <a
            href={hashForBrowserSession(sessionId)}
            className="ml-auto shrink-0 inline-flex items-center gap-1 text-ink-faint hover:text-ink transition-colors"
          >
            <SquareArrowOutUpRight size={11} aria-hidden />
            open in browser tab
          </a>
        ) : null}
      </div>
      {running && message.output == null ? (
        <p className="px-3 py-2 font-mono text-[12px] lowercase text-ink-faint">
          running...
        </p>
      ) : screenshot?.dataUrl ? (
        <div className="p-3">
          <img
            src={screenshot.dataUrl}
            alt={`capture of ${screenshot.url}`}
            className="block max-w-full max-h-80 border border-rule"
          />
        </div>
      ) : message.output != null ? (
        <div className="max-h-64 overflow-auto">
          <JsonHighlight code={formatJson(message.output)} />
        </div>
      ) : (
        <p className="px-3 py-2 font-mono text-[12px] lowercase text-ink-ghost">
          no result
        </p>
      )}
    </div>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isBrowserFunction(message.functionId)) return null
  if (message.pendingApproval) return null
  return <BrowserCallView message={message} />
}

/** `browser::*` calls have no bespoke pending preview. */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const BrowserToolView = {
  isBrowserFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
