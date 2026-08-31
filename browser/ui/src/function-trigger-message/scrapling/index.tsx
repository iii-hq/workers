import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'
import { InfraErrorView, parseInfraErrorDisplay } from '../../lib/errors'
import { FunctionIdLabel } from '..'
import { CrawlPreview, CrawlView } from './CrawlView'
import { FetchPreview, FetchView } from './FetchView'
import { MarkdownView } from './MarkdownView'
import { ExtractView, FindSimilarView, QueryView } from './ParseViews'
import {
  ELEMENT_SEARCH_FUNCTION_IDS,
  FETCH_FUNCTION_IDS,
  isScraplingFunction,
  SESSION_GATED_FUNCTION_IDS,
  unwrapEnvelope,
} from './parsers'
import { ScreenshotPreview, ScreenshotView } from './ScreenshotView'
import { DescribeView, ElementsView } from './SearchViews'
import {
  SessionClosePreview,
  SessionCloseView,
  SessionFetchPreview,
  SessionFetchView,
  SessionListPreview,
  SessionListView,
  SessionOpenPreview,
  SessionOpenView,
} from './SessionViews'

const PROXY_KEYS = new Set(['proxy', 'proxies', 'proxy_auth'])
const PROXY_URL_USERINFO = /(https?:\/\/)[^\s/?#]*@/gi

function redactProxyValues(
  value: unknown,
  seen: WeakSet<object> = new WeakSet(),
): unknown {
  if (typeof value === 'string') {
    return value.replace(PROXY_URL_USERINFO, '$1[redacted]@')
  }
  if (value === null || typeof value !== 'object') return value
  if (seen.has(value)) return '[circular]'
  seen.add(value)
  try {
    if (Array.isArray(value)) {
      return value.map((entry) => redactProxyValues(entry, seen))
    }
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, entry]) => [
        key,
        PROXY_KEYS.has(key) ? '[redacted]' : redactProxyValues(entry, seen),
      ]),
    )
  } catch {
    return '[redacted]'
  } finally {
    seen.delete(value)
  }
}

function tryRender(
  message: FunctionTriggerMessage,
  host?: Host,
): React.ReactNode | null {
  if (!isScraplingFunction(message.functionId) || message.pendingApproval) {
    return null
  }

  const running = !!message.running
  const rawOutput = message.output
  const errorDisplay =
    !running && rawOutput != null
      ? parseInfraErrorDisplay(redactProxyValues(rawOutput))
      : null
  if (errorDisplay) return <InfraErrorView display={errorDisplay} />

  const input = unwrapEnvelope(message.input)
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined

  if (FETCH_FUNCTION_IDS.has(message.functionId)) {
    return FetchView({
      functionId: message.functionId,
      input,
      output,
      running,
      host,
    })
  }
  if (ELEMENT_SEARCH_FUNCTION_IDS.has(message.functionId)) {
    return ElementsView({
      functionId: message.functionId,
      input,
      output,
      running,
    })
  }
  switch (message.functionId) {
    case 'browser::screenshot-url':
      return ScreenshotView({ input, output, running })
    case 'browser::extract':
      return ExtractView({ input, output, running })
    case 'browser::css':
    case 'browser::xpath':
    case 'browser::regex':
      return QueryView({
        functionId: message.functionId,
        input,
        output,
        running,
      })
    case 'browser::find-similar':
      return FindSimilarView({ input, output, running })
    case 'browser::describe':
      return DescribeView({ input, output, running })
    case 'browser::to-markdown':
      return MarkdownView({ input, output, running })
    case 'browser::session-open':
      return SessionOpenView({ input, output, running })
    case 'browser::session-fetch':
      return SessionFetchView({ input, output, running, host })
    case 'browser::session-close':
      return SessionCloseView({ input, output, running })
    case 'browser::session-list':
      return SessionListView({ input, output, running })
    case 'browser::crawl':
      return CrawlView({ input, output, running })
  }
}

function tryRenderPreview(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (!isScraplingFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  if (FETCH_FUNCTION_IDS.has(message.functionId)) {
    return FetchPreview({ functionId: message.functionId, input })
  }
  if (message.functionId === 'browser::screenshot-url') {
    return ScreenshotPreview({ input })
  }
  if (SESSION_GATED_FUNCTION_IDS.has(message.functionId)) {
    switch (message.functionId) {
      case 'browser::session-open':
        return SessionOpenPreview({ input })
      case 'browser::session-fetch':
        return SessionFetchPreview({ input })
      case 'browser::session-close':
        return SessionClosePreview({ input })
      case 'browser::session-list':
        return SessionListPreview({ input })
    }
  }
  if (message.functionId === 'browser::crawl') return CrawlPreview({ input })
  return null
}

export function createScraplingRenderer(
  host: Host,
): FunctionTriggerRenderer {
  const render = (message: FunctionTriggerMessage) => tryRender(message, host)
  return {
    id: 'browser/page.js#scraping-calls',
    isMatch: isScraplingFunction,
    tryRender: render,
    tryRenderRunning: render,
    tryRenderPreview,
    FunctionIdLabel,
    redactRaw: redactProxyValues,
  }
}
