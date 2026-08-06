/**
 * Injected function-trigger renderer for every `browser::*` call —
 * registered through `host.functionTriggers`, so it dispatches BEFORE the
 * console's built-in families and owns how browser calls render in chat and
 * in the traces span tab. Ported from the console's `components/chat/browser`
 * family; the "open in browser tab" link now points at the injected page
 * (`#/ext/browser`), and shared-infra errors render through the ported
 * `InfraErrorView` instead of the sandbox family.
 */

import {
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'
import {
  browserSessionIdFromCall,
  isBrowserFunction,
  parseScreenshotOutput,
} from '../lib/browser'
import { InfraErrorView, parseInfraErrorDisplay } from '../lib/errors'
import { ExternalLink } from '../lib/icons'
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

/** The injected page's route — where "open in browser tab" navigates. */
const BROWSER_PAGE_HASH = '#/ext/browser'

/**
 * Header label for `browser::*` ids: dims the namespace prefix so the op
 * (`navigate`, `act`, …) reads clearly.
 */
function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('browser::')) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  const tail = functionId.slice('browser::'.length)
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>browser::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{tail}</span>
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
    <div className="br-ui-shot">
      <img
        src={screenshot.dataUrl}
        alt={`capture of ${screenshot.url}`}
        className="br-ui-shot-img"
      />
    </div>
  )
}

/**
 * Per-function pretty body; null when the function is unknown or its
 * payload doesn't parse, in which case the caller falls back to the
 * decoded-JSON rendering.
 */
function renderBody(message: FunctionTriggerMessage): React.ReactNode | null {
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
 * "open in browser tab" affordance (routes to `#/ext/browser`), then the
 * per-function pretty body. Unknown or unparseable payloads fall back to the
 * decoded result as clamped JSON.
 */
function BrowserCallView({ message }: { message: FunctionTriggerMessage }) {
  const sessionId = browserSessionIdFromCall(message.input, message.output)
  const running = !!message.running

  const body = !running && message.output != null ? renderBody(message) : null
  const fallback =
    !body && message.output != null ? decodeBrowserResult(message.output) : null

  return (
    <div className="br-ui-call">
      <div className="br-ui-call-head">
        {sessionId ? (
          <span className="br-ui-call-session">
            session <span className="br-ui-call-sid">{sessionId}</span>
          </span>
        ) : (
          <span className="br-ui-call-session">browser</span>
        )}
        {sessionId ? (
          <a href={BROWSER_PAGE_HASH} className="br-ui-call-link">
            <ExternalLink size={11} aria-hidden />
            open in browser tab
          </a>
        ) : null}
      </div>
      {running && message.output == null ? (
        <p className="br-ui-call-running">running...</p>
      ) : body ? (
        body
      ) : fallback != null ? (
        <div className="br-ui-json">
          <JsonHighlight code={formatJson(fallback)} />
        </div>
      ) : (
        <p className="br-ui-empty-line">no result</p>
      )}
    </div>
  )
}

function renderCall(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isBrowserFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const running = !!message.running
  const rawOutput = message.output
  // Shared infra errors (gate denials, dispatch policy, function_error
  // envelopes). Browser success payloads never look denial-shaped: results
  // are `ok`-flagged structs or `{content, details}` envelopes.
  const errorDisplay =
    !running && rawOutput != null ? parseInfraErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <InfraErrorView display={errorDisplay} />
  }
  return <BrowserCallView message={message} />
}

export function createBrowserRenderer(_host: Host): FunctionTriggerRenderer {
  return {
    id: 'browser/page.js#calls',
    isMatch: isBrowserFunction,
    tryRender: (message) => renderCall(message),
    tryRenderRunning: (message) => renderCall(message),
    tryRenderPreview: () => null,
    FunctionIdLabel,
  }
}
