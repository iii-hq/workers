import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { CrawlPreview, CrawlView } from './CrawlView'
import { FetchPreview, FetchView } from './FetchView'
import { MarkdownView } from './MarkdownView'
import { ExtractView, FindSimilarView, QueryView } from './ParseViews'
import {
  crawlRequestSchema,
  ELEMENT_SEARCH_FUNCTION_IDS,
  FETCH_FUNCTION_IDS,
  fetchRequestSchema,
  isScraplingFunction,
  SESSION_GATED_FUNCTION_IDS,
  safeParseRequest,
  screenshotRequestSchema,
  sessionOpenRequestSchema,
  unwrapEnvelope,
} from './parsers'
import { ScreenshotPreview, ScreenshotView } from './ScreenshotView'
import { DescribeView, ElementsView } from './SearchViews'
import {
  SessionCloseView,
  SessionFetchPreview,
  SessionFetchView,
  SessionListView,
  SessionOpenPreview,
  SessionOpenView,
} from './SessionViews'

/**
 * Header label for `scrapling::*` ids — dims the namespace prefix so the op
 * (`fetch`, `css`, …) reads clearly. Mirrors `WebFunctionIdLabel`.
 */
export function ScraplingFunctionIdLabel({
  functionId,
}: {
  functionId: string
}) {
  if (!functionId.startsWith('scrapling::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('scrapling::'.length)
  return (
    <>
      <span className="text-ink-faint">scrapling::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isScraplingFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const running = !!message.running
  const rawOutput = message.output

  // Shared infra errors (gate denials, dispatch policy, function_error
  // envelopes). Scrapling success payloads never look denial-shaped — `status`
  // is a number and per-url bulk errors sit inside `results` rows — so the
  // parser is safe to run on every completed output.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  const input = unwrapEnvelope(message.input)
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined

  if (FETCH_FUNCTION_IDS.has(message.functionId)) {
    return (
      <FetchView
        functionId={message.functionId}
        input={input}
        output={output}
        running={running}
      />
    )
  }
  if (ELEMENT_SEARCH_FUNCTION_IDS.has(message.functionId)) {
    return (
      <ElementsView
        functionId={message.functionId}
        input={input}
        output={output}
        running={running}
      />
    )
  }
  switch (message.functionId) {
    case 'scrapling::screenshot':
      return <ScreenshotView input={input} output={output} running={running} />
    case 'scrapling::extract':
      return <ExtractView input={input} output={output} running={running} />
    case 'scrapling::css':
    case 'scrapling::xpath':
    case 'scrapling::regex':
      return (
        <QueryView
          functionId={message.functionId}
          input={input}
          output={output}
          running={running}
        />
      )
    case 'scrapling::find-similar':
      return <FindSimilarView input={input} output={output} running={running} />
    case 'scrapling::describe':
      return <DescribeView input={input} output={output} running={running} />
    case 'scrapling::to-markdown':
      return <MarkdownView input={input} output={output} running={running} />
    case 'scrapling::session-open':
      return <SessionOpenView input={input} output={output} running={running} />
    case 'scrapling::session-fetch':
      return (
        <SessionFetchView input={input} output={output} running={running} />
      )
    case 'scrapling::session-close':
      return <SessionCloseView output={output} />
    case 'scrapling::session-list':
      return <SessionListView output={output} />
    case 'scrapling::crawl':
      return <CrawlView input={input} output={output} running={running} />
    default:
      return null
  }
}

/** Approval previews for the gated network calls; parse-only ops are
 *  auto-allowed and never sit in the gate. */
function tryRenderPreview(
  message: FunctionCallMessage,
): React.ReactNode | null {
  if (!isScraplingFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  // Parse-check BEFORE building an element: the preview components render
  // null for an unparseable request, but a non-null element here makes
  // FunctionCallCard suppress its raw request pane (the null contract).
  // SessionFetchPreview is exempt — it renders a header for any input.
  if (FETCH_FUNCTION_IDS.has(message.functionId)) {
    return safeParseRequest(fetchRequestSchema, input) ? (
      <FetchPreview functionId={message.functionId} input={input} />
    ) : null
  }
  if (message.functionId === 'scrapling::screenshot') {
    return safeParseRequest(screenshotRequestSchema, input) ? (
      <ScreenshotPreview input={input} />
    ) : null
  }
  if (SESSION_GATED_FUNCTION_IDS.has(message.functionId)) {
    if (message.functionId === 'scrapling::session-open') {
      return safeParseRequest(sessionOpenRequestSchema, input) ? (
        <SessionOpenPreview input={input} />
      ) : null
    }
    return <SessionFetchPreview input={input} />
  }
  if (message.functionId === 'scrapling::crawl') {
    return safeParseRequest(crawlRequestSchema, input) ? (
      <CrawlPreview input={input} />
    ) : null
  }
  return null
}

export const ScraplingToolView = {
  isScraplingFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
