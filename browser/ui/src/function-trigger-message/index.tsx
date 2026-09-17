/**
 * Injected function-trigger renderer for every `browser::*` call —
 * registered through `host.functionTriggers`, so it dispatches BEFORE the
 * console's built-in families and owns how browser calls render in chat and
 * in the traces span tab. Ported from the console's `components/chat/browser`
 * family; the "open in browser tab" action opens the injected page through
 * `host.panels.open`, and shared-infra errors render through the ported
 * `InfraErrorView` instead of the sandbox family.
 */

import {
  EmptyState,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  JsonHighlight,
  Skeleton,
} from '@iii-dev/console-ui'
import { ExternalLink } from 'lucide-react'
import {
  browserSessionIdFromCall,
  isBrowserFunction,
  parseScreenshotOutput,
} from '../lib/browser'
import { InfraErrorView, parseInfraErrorDisplay } from '../lib/errors'
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

/**
 * Header label for `browser::*` ids: dims the namespace prefix so the op
 * (`navigate`, `act`, …) reads clearly.
 */
export function FunctionIdLabel({ functionId }: { functionId: string }) {
  const [ns, op] = functionId.startsWith('browser::')
    ? ['browser::', functionId.slice('browser::'.length)]
    : ['', functionId]
  return (
    <>
      {ns ? <span style={{ color: 'var(--color-ink-faint)' }}>{ns}</span> : null}
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{op}</span>
    </>
  )
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

function renderScreenshot(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (
    message.functionId !== 'browser::screenshot' ||
    message.pendingApproval ||
    message.running ||
    message.output == null
  ) {
    return null
  }
  if (parseInfraErrorDisplay(message.output)) return null
  const screenshot = parseScreenshotOutput(message.output)
  if (!screenshot?.dataUrl) return null
  return <ScreenshotBody output={message.output} />
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
 * "open in browser tab" affordance (`host.panels.open` to the page), then the
 * per-function pretty body. Unknown or unparseable payloads fall back to the
 * decoded result as clamped JSON.
 */
function BrowserCallView({
  host,
  message,
}: {
  host: Host
  message: FunctionTriggerMessage
}) {
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
          <button
            type="button"
            className="br-ui-call-link"
            onClick={() => host.panels?.open({ pageId: 'browser' })}
          >
            <ExternalLink size={16} aria-hidden />
            open in browser tab
          </button>
        ) : null}
      </div>
      {running && message.output == null ? (
        <div className="br-ui-call-running" aria-busy aria-label="Running">
          <Skeleton className="br-ui-skel" />
        </div>
      ) : body ? (
        body
      ) : fallback != null ? (
        <div className="br-ui-json">
          <JsonHighlight code={JSON.stringify(fallback, null, 2) ?? 'null'} />
        </div>
      ) : (
        <EmptyState
          title="No result"
          description="The call finished without returning a payload."
        />
      )}
    </div>
  )
}

function renderCall(
  host: Host,
  message: FunctionTriggerMessage,
): React.ReactNode | null {
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
  return <BrowserCallView host={host} message={message} />
}

export function createBrowserRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'browser/page.js#calls',
    isMatch: isBrowserFunction,
    tryRender: (message) => renderCall(host, message),
    tryRenderRunning: (message) => renderCall(host, message),
    tryRenderPreview: () => null,
    FunctionIdLabel,
  }
}

/**
 * Focused renderer that promotes successful screenshots into the chat flow.
 * Keeping it separate means other browser calls retain the compact card and
 * a malformed/error response safely falls through to the general renderer.
 */
export function createBrowserScreenshotRenderer(): FunctionTriggerRenderer {
  return {
    id: 'browser/page.js#screenshot-display',
    isMatch: (functionId) => functionId === 'browser::screenshot',
    tryRender: renderScreenshot,
    tryRenderRunning: () => null,
    tryRenderPreview: () => null,
    FunctionIdLabel,
    metadata: { display: true },
  }
}
