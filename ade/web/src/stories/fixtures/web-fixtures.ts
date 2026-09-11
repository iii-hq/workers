import type { FunctionTriggerMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionTriggerMessage>,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId,
    input,
    output,
    durationMs: 134,
    createdAt: now,
    ...extra,
  }
}

const todosBody = JSON.stringify({ ok: true, todos: [] })

/** Realistic JSON success that mirrors the screenshot's curl-against-localhost
 * shape. Body is text by default and parsed to pretty JSON by content-type. */
export const webFetchJsonSuccess = base(
  'web-fetch-json',
  'web::fetch',
  { method: 'GET', url: 'http://127.0.0.1:3111/todos' },
  wrapHarness({
    ok: true,
    status: 200,
    status_text: 'OK',
    headers: {
      'access-control-allow-origin': '*',
      'content-length': '22',
      'content-type': 'application/json',
      date: 'Wed, 27 May 2026 21:56:43 GMT',
      vary: 'origin, access-control-request-method, access-control-request-headers',
    },
    body: todosBody,
    response_format: 'text',
    bytes_truncated: false,
  }),
)

export const webFetchJsonExplicitFormat = base(
  'web-fetch-json-explicit',
  'web::fetch',
  {
    method: 'GET',
    url: 'https://api.example.com/v1/items',
    response_format: 'json',
    timeout_ms: 5_000,
  },
  {
    ok: true,
    status: 200,
    status_text: 'OK',
    headers: { 'content-type': 'application/json' },
    body: '{"items":[{"id":1,"name":"a"},{"id":2,"name":"b"}]}',
    response_format: 'json',
    bytes_truncated: false,
    json: {
      items: [
        { id: 1, name: 'a' },
        { id: 2, name: 'b' },
      ],
    },
    redirect_chain: ['https://example.com/v1/items'],
  },
)

export const webFetchTextHtml = base(
  'web-fetch-html',
  'web::fetch',
  { method: 'GET', url: 'https://example.com/' },
  wrapHarness({
    ok: true,
    status: 200,
    status_text: 'OK',
    headers: {
      'content-type': 'text/html; charset=UTF-8',
      'content-length': '1256',
    },
    body: '<!doctype html>\n<html>\n  <head><title>Example</title></head>\n  <body><h1>Example Domain</h1></body>\n</html>\n',
    response_format: 'text',
    bytes_truncated: false,
  }),
)

export const webFetchTruncated = base(
  'web-fetch-truncated',
  'web::fetch',
  {
    method: 'GET',
    url: 'https://feeds.example.com/huge.json',
    max_bytes: 1024,
  },
  {
    ok: true,
    status: 200,
    status_text: 'OK',
    headers: { 'content-type': 'application/json' },
    body: '{"items":[…',
    response_format: 'text',
    bytes_truncated: true,
  },
)

export const webFetchBase64 = base(
  'web-fetch-base64',
  'web::fetch',
  {
    method: 'GET',
    url: 'https://cdn.example.com/icon.png',
    response_format: 'base64',
  },
  {
    ok: true,
    status: 200,
    status_text: 'OK',
    headers: {
      'content-type': 'image/png',
      'content-length': '512',
    },
    body: 'iVBORw0KGgoAAAANSUhEUgAAAAUAAAAFCAYAAACNbyblAAAAHElEQVQI12P4//8/w38GIAXDIBKE0DHxgljNBAAO9TXL0Y4OHwAAAABJRU5ErkJggg==',
    response_format: 'base64',
    bytes_truncated: false,
  },
)

export const webFetchNotFound = base(
  'web-fetch-404',
  'web::fetch',
  { method: 'GET', url: 'https://api.example.com/missing' },
  {
    ok: true,
    status: 404,
    status_text: 'Not Found',
    headers: { 'content-type': 'application/json' },
    body: '{"error":"not_found"}',
    response_format: 'text',
    bytes_truncated: false,
  },
)

export const webFetchServerError = base(
  'web-fetch-500',
  'web::fetch',
  { method: 'POST', url: 'https://api.example.com/orders' },
  {
    ok: true,
    status: 502,
    status_text: 'Bad Gateway',
    headers: { 'content-type': 'text/plain' },
    body: 'upstream timed out',
    response_format: 'text',
    bytes_truncated: false,
  },
)

export const webFetchPending = base(
  'web-fetch-pending',
  'web::fetch',
  {
    method: 'POST',
    url: 'https://api.example.com/orders',
    headers: { 'content-type': 'application/json' },
    json: { sku: 'A-12', qty: 3 },
    timeout_ms: 8_000,
  },
  undefined,
  { pendingApproval: true },
)

export const webFetchRunning = base(
  'web-fetch-running',
  'web::fetch',
  { method: 'GET', url: 'https://example.com/' },
  undefined,
  { running: true },
)

export const webFetchTimeoutError = base(
  'web-fetch-err-timeout',
  'web::fetch',
  { method: 'GET', url: 'https://slow.example.com/', timeout_ms: 1_000 },
  {
    ok: false,
    error: 'timeout',
    message:
      'request timed out after 1000ms (raise timeout_ms or shrink response with smaller max_bytes)',
  },
)

export const webFetchBlockedHostError = base(
  'web-fetch-err-blocked',
  'web::fetch',
  { method: 'GET', url: 'http://169.254.169.254/' },
  {
    ok: false,
    error: 'blocked_host',
    message:
      'host 169.254.169.254 resolves to a link-local address blocked by the SSRF policy',
  },
)

export const webFetchTooLargeError = base(
  'web-fetch-err-too-large',
  'web::fetch',
  { method: 'GET', url: 'https://example.com/big', max_bytes: 1024 },
  {
    ok: false,
    error: 'too_large',
    message: 'response exceeded max_bytes (1024) before completion',
    status: 200,
  },
)

export const webFetchTransportError = base(
  'web-fetch-err-transport',
  'web::fetch',
  { method: 'GET', url: 'https://no-such-host.example.invalid/' },
  {
    ok: false,
    error: 'transport_error',
    message: 'dns lookup failed for host',
  },
)

export const webFetchInvalidUrlError = base(
  'web-fetch-err-invalid-url',
  'web::fetch',
  { method: 'GET', url: 'not a url' },
  {
    ok: false,
    error: 'invalid_url',
    message: 'url must be an absolute http(s):// URL',
  },
)

/** Fail-closed dispatch-policy denial — the agent's allow-list omits web::fetch.
 * The harness rejects the call before it runs; renders as the actionable
 * DispatchDeniedView (not the raw "invocation failed" string). This is the #1
 * workflow-node failure: a node authored without `agent.functions: ["web::fetch"]`. */
export const webFetchDispatchDenied = base(
  'web-fetch-err-dispatch-denied',
  'web::fetch',
  { method: 'GET', url: 'https://news.ycombinator.com' },
  {
    error: {
      kind: 'function_error',
      message:
        "function web::fetch is not permitted by this agent's dispatch policy (no allow-glob match or a deny-glob match)",
      content: [
        {
          type: 'text',
          text: "function web::fetch is not permitted by this agent's dispatch policy",
        },
      ],
    },
  },
)

export const webFixtures = [
  webFetchJsonSuccess,
  webFetchJsonExplicitFormat,
  webFetchTextHtml,
  webFetchTruncated,
  webFetchBase64,
  webFetchNotFound,
  webFetchServerError,
  webFetchPending,
  webFetchRunning,
  webFetchTimeoutError,
  webFetchBlockedHostError,
  webFetchTooLargeError,
  webFetchTransportError,
  webFetchInvalidUrlError,
  webFetchDispatchDenied,
] as const
