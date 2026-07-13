import { SquareArrowOutUpRight } from 'lucide-react'
import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import { hashForBrowserSession } from '@/hooks/use-hash-route'
import {
  browserSessionIdFromCall,
  isBrowserFunction,
  parseScreenshotOutput,
} from '@/lib/browser'
import { JsonHighlight } from '@/lib/syntax'
import type { FunctionCallMessage } from '@/types/chat'
import {
  ActView,
  ConsoleReadView,
  DomReadView,
  EvaluateView,
  HistoryView,
  NavigateView,
  NetworkReadView,
  SessionListView,
  SessionStartView,
  SessionStopView,
  SnapshotView,
  StylesReadView,
  StylesWriteView,
} from './BrowserViews'
import { decodeBrowserResult } from './parsers'

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

function ScreenshotBody({ output }: { output: unknown }) {
  const screenshot = parseScreenshotOutput(output)
  if (!screenshot?.dataUrl) return null
  return (
    <div className="p-3">
      <img
        src={screenshot.dataUrl}
        alt={`capture of ${screenshot.url}`}
        className="block max-w-full max-h-80 border border-rule"
      />
    </div>
  )
}

/**
 * Per-function pretty body; null when the function is unknown or its
 * payload doesn't parse, in which case the caller falls back to the
 * decoded-JSON rendering.
 */
function renderBody(message: FunctionCallMessage): React.ReactNode | null {
  const input = message.input
  const output = message.output
  switch (message.functionId) {
    case 'browser::snapshot':
      return <SnapshotView output={output} />
    case 'browser::sessions::start':
      return <SessionStartView output={output} />
    case 'browser::sessions::stop':
      return <SessionStopView output={output} />
    case 'browser::sessions::list':
      return <SessionListView output={output} />
    case 'browser::navigate':
      return <NavigateView output={output} />
    case 'browser::console::read':
      return <ConsoleReadView input={input} output={output} />
    case 'browser::network::read':
      return <NetworkReadView input={input} output={output} />
    case 'browser::act':
      return <ActView input={input} output={output} />
    case 'browser::history':
      return <HistoryView input={input} output={output} />
    case 'browser::styles::read':
      return <StylesReadView output={output} />
    case 'browser::styles::write':
      return <StylesWriteView input={input} output={output} />
    case 'browser::dom::read':
      return <DomReadView output={output} />
    case 'browser::evaluate':
      return <EvaluateView input={input} output={output} />
    case 'browser::screenshot':
      return <ScreenshotBody output={output} />
    default:
      return null
  }
}

/**
 * Terminal view for a `browser::*` call: the owning session with an
 * "open in browser tab" affordance (routes to `#/browser/<session_id>`),
 * then the per-function pretty body. Unknown or unparseable payloads fall
 * back to the decoded result as clamped JSON; the raw json tab keeps the
 * untouched request/response panes.
 */
function BrowserCallView({ message }: { message: FunctionCallMessage }) {
  const sessionId = browserSessionIdFromCall(message.input, message.output)
  const running = !!message.running

  const body = !running && message.output != null ? renderBody(message) : null
  // Fallback decode only when no pretty body renders (the normal case has a
  // body, so the full envelope parse is skipped there).
  const fallback =
    !body && message.output != null ? decodeBrowserResult(message.output) : null

  return (
    <div className="flex flex-col bg-bg">
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
        <p className="px-3 py-2 font-mono text-[12px] lowercase text-ink-faint animate-pulse">
          running...
        </p>
      ) : body ? (
        body
      ) : fallback != null ? (
        <div className="max-h-64 overflow-auto">
          <JsonHighlight code={formatJson(fallback)} />
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

  const running = !!message.running
  const rawOutput = message.output
  // Shared infra errors (gate denials, dispatch policy, function_error
  // envelopes). Browser success payloads never look denial-shaped: results
  // are `ok`-flagged structs or `{content, details}` envelopes.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }
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
