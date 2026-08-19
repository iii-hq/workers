/**
 * Zod schemas + helpers for the browser scraping function family.
 *
 * Wire source:
 *   workers/scrapling/src/schemas.py -> FUNCTIONS (request/response schemas)
 *   workers/scrapling/src/core.py    -> serialize_page / op_* return shapes
 *
 * Fetch responses (fetch / stealthy-fetch / dynamic-fetch) are one of:
 *   single page  { status, url, headers, cookies, encoding, extracted?,
 *                  captured_xhr?, html? }
 *   bulk         { results: [ page | { url, error } ] }   (when called with `urls`)
 *
 * Schemas are non-strict so additive wire fields don't break the UI.
 */
import { z } from 'zod'
import { decodeBrowserResult } from '../../lib/browser'

export const unwrapEnvelope = decodeBrowserResult

export const SCRAPLING_FUNCTION_IDS = [
  'browser::fetch',
  'browser::stealthy-fetch',
  'browser::dynamic-fetch',
  'browser::screenshot-url',
  'browser::extract',
  'browser::css',
  'browser::xpath',
  'browser::regex',
  'browser::find-similar',
  'browser::find',
  'browser::find-by-text',
  'browser::find-by-regex',
  'browser::describe',
  'browser::to-markdown',
  'browser::session-open',
  'browser::session-fetch',
  'browser::session-close',
  'browser::session-list',
  'browser::crawl',
] as const
export type ScraplingFunctionId = (typeof SCRAPLING_FUNCTION_IDS)[number]

/** find / find-by-text / find-by-regex all return `{count, items:[element]}`. */
export const ELEMENT_SEARCH_FUNCTION_IDS: ReadonlySet<string> = new Set([
  'browser::find',
  'browser::find-by-text',
  'browser::find-by-regex',
])

/** Session operations require approval in the browser worker permissions. */
export const SESSION_GATED_FUNCTION_IDS: ReadonlySet<string> = new Set([
  'browser::session-open',
  'browser::session-fetch',
  'browser::session-close',
  'browser::session-list',
])

const SCRAPLING_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  SCRAPLING_FUNCTION_IDS,
)

export function isScraplingFunction(id: string): id is ScraplingFunctionId {
  return SCRAPLING_FUNCTION_ID_SET.has(id)
}

export const FETCH_FUNCTION_IDS: ReadonlySet<string> = new Set([
  'browser::fetch',
  'browser::stealthy-fetch',
  'browser::dynamic-fetch',
])

/** Which fetch tier a function runs on — drives the engine chip. */
export function fetchEngineLabel(functionId: string): string {
  switch (functionId) {
    case 'browser::stealthy-fetch':
      return 'stealth'
    case 'browser::dynamic-fetch':
      return 'chromium'
    default:
      return 'http'
  }
}

/* ---------------- selectors (shared by fetch + parse ops) ---------------- */

export const selectorSpecSchema = z.object({
  name: z.string(),
  css: z.string().optional(),
  xpath: z.string().optional(),
  regex: z.string().optional(),
  attr: z.string().optional(),
  html: z.boolean().optional(),
  all: z.boolean().optional(),
})
export type SelectorSpec = z.infer<typeof selectorSpecSchema>

/* ---------------- fetch / stealthy-fetch / dynamic-fetch ---------------- */

export const fetchRequestSchema = z.object({
  url: z.string().min(1).optional(),
  urls: z.array(z.string().min(1)).optional(),
  method: z.string().optional(),
  impersonate: z.string().optional(),
  proxy: z.string().optional(),
  headless: z.boolean().optional(),
  network_idle: z.boolean().optional(),
  solve_cloudflare: z.boolean().optional(),
  real_chrome: z.boolean().optional(),
  cdp_url: z.string().optional(),
  wait_selector: z.string().optional(),
  timeout: z.number().optional(),
  selectors: z.array(selectorSpecSchema).optional(),
  include_html: z.boolean().optional(),
  format: z.string().optional(),
  main_content_only: z.boolean().optional(),
}).refine(
  (request) =>
    (request.url?.length ?? 0) > 0 || (request.urls?.length ?? 0) > 0,
)
export type FetchRequest = z.infer<typeof fetchRequestSchema>

export const pageResultSchema = z.object({
  status: z.number().nullable().optional(),
  url: z.string().optional(),
  headers: z.record(z.string(), z.unknown()).optional(),
  cookies: z.record(z.string(), z.unknown()).optional(),
  encoding: z.string().nullable().optional(),
  extracted: z.record(z.string(), z.unknown()).optional(),
  captured_xhr: z.array(z.unknown()).optional(),
  html: z.string().optional(),
  /** markdown/text render when `format` was requested */
  content: z.string().optional(),
  format: z.string().optional(),
  /** bulk per-url failure rows are `{ url, error }` */
  error: z.string().optional(),
}).refine((result) => Object.keys(result).length > 0)
export type PageResult = z.infer<typeof pageResultSchema>

export const bulkResponseSchema = z.object({
  results: z.array(pageResultSchema),
})
export type BulkResponse = z.infer<typeof bulkResponseSchema>

/** Bulk first: the loose page schema would match (and strip) a bulk payload. */
export const fetchResponseSchema = z.union([
  bulkResponseSchema,
  pageResultSchema,
])
export type FetchResponse = z.infer<typeof fetchResponseSchema>

/* ---------------- screenshot ---------------- */

export const screenshotRequestSchema = z.object({
  url: z.string(),
  fetcher: z.string().optional(),
  proxy: z.string().optional(),
  full_page: z.boolean().optional(),
  format: z.string().optional(),
})
export type ScreenshotRequest = z.infer<typeof screenshotRequestSchema>

/** Harness content blocks: image tiles + a trailing text caption. */
const screenshotBlockSchema = z.object({
  type: z.string(),
  mime: z.string().optional(),
  data: z.string().optional(),
  text: z.string().optional(),
})

export const screenshotResponseSchema = z
  .object({
    content: z.array(screenshotBlockSchema),
    mime: z.string().optional(),
    url: z.string().optional(),
  })
  .refine((response) =>
    response.content.some(
      (block) => block.type === 'image' && (block.data?.length ?? 0) > 0,
    ),
  )
export type ScreenshotResponse = z.infer<typeof screenshotResponseSchema>

/* ---------------- parse-only ops ---------------- */

export const extractRequestSchema = z.object({
  html: z.string(),
  selectors: z.array(selectorSpecSchema),
  adaptive: z.boolean().optional(),
  adaptive_domain: z.string().optional(),
})
export type ExtractRequest = z.infer<typeof extractRequestSchema>

export const extractResponseSchema = z.object({
  extracted: z.record(z.string(), z.unknown()),
})
export type ExtractResponse = z.infer<typeof extractResponseSchema>

export const queryRequestSchema = z.object({
  html: z.string(),
  query: z.string().optional(),
  pattern: z.string().optional(),
  first: z.boolean().optional(),
  attr: z.string().optional(),
  adaptive: z.boolean().optional(),
  identifier: z.string().optional(),
  adaptive_domain: z.string().optional(),
})
export type QueryRequest = z.infer<typeof queryRequestSchema>

/** `attr` misses inside an `all` list come back as null items. */
export const queryResponseSchema = z.object({
  result: z.union([z.array(z.string().nullable()), z.string(), z.null()]),
})
export type QueryResponse = z.infer<typeof queryResponseSchema>

export const findSimilarRequestSchema = z.object({
  html: z.string(),
  anchor: z.string(),
  similarity_threshold: z.number().optional(),
  match_text: z.boolean().optional(),
  selectors: z.array(selectorSpecSchema).optional(),
})
export type FindSimilarRequest = z.infer<typeof findSimilarRequestSchema>

export const findSimilarResponseSchema = z.object({
  count: z.number(),
  items: z.array(z.record(z.string(), z.unknown())),
})
export type FindSimilarResponse = z.infer<typeof findSimilarResponseSchema>

/* ---------------- element search (find / find-by-text / find-by-regex) ---- */

export const elementSchema = z.object({
  tag: z.string().optional(),
  text: z.string().optional(),
  html: z.string().optional(),
  attrs: z.record(z.string(), z.unknown()).optional(),
  css: z.string().optional(),
  xpath: z.string().optional(),
})
export type ScrapedElement = z.infer<typeof elementSchema>

export const elementsResponseSchema = z.object({
  count: z.number(),
  items: z.array(elementSchema),
})
export type ElementsResponse = z.infer<typeof elementsResponseSchema>

export const findRequestSchema = z.object({
  html: z.string(),
  tag: z.union([z.string(), z.array(z.string())]).optional(),
  attrs: z.record(z.string(), z.unknown()).optional(),
  text_regex: z.string().optional(),
  first: z.boolean().optional(),
  limit: z.number().optional(),
})
export type FindRequest = z.infer<typeof findRequestSchema>

export const findByTextRequestSchema = z.object({
  html: z.string(),
  text: z.string(),
  partial: z.boolean().optional(),
  case_sensitive: z.boolean().optional(),
  first: z.boolean().optional(),
})
export type FindByTextRequest = z.infer<typeof findByTextRequestSchema>

export const findByRegexRequestSchema = z.object({
  html: z.string(),
  pattern: z.string(),
  case_sensitive: z.boolean().optional(),
  first: z.boolean().optional(),
})
export type FindByRegexRequest = z.infer<typeof findByRegexRequestSchema>

/* ---------------- describe ---------------- */

export const describeRequestSchema = z.object({
  html: z.string(),
  query: z.string(),
  kind: z.string().optional(),
})
export type DescribeRequest = z.infer<typeof describeRequestSchema>

export const describeElementSchema = elementSchema.extend({
  full_css: z.string().optional(),
  full_xpath: z.string().optional(),
  classes: z.array(z.string()).optional(),
  parent_tag: z.string().nullable().optional(),
  children: z.number().optional(),
  siblings: z.number().optional(),
})
export type DescribeElement = z.infer<typeof describeElementSchema>

export const describeResponseSchema = z.object({
  found: z.boolean(),
  element: describeElementSchema.optional(),
})
export type DescribeResponse = z.infer<typeof describeResponseSchema>

/* ---------------- to-markdown ---------------- */

export const markdownRequestSchema = z.object({
  html: z.string(),
  format: z.string().optional(),
  css_selector: z.string().optional(),
  main_content_only: z.boolean().optional(),
})
export type MarkdownRequest = z.infer<typeof markdownRequestSchema>

export const markdownResponseSchema = z.object({
  format: z.string(),
  content: z.string(),
})
export type MarkdownResponse = z.infer<typeof markdownResponseSchema>

/* ---------------- sessions ---------------- */

export const sessionOpenRequestSchema = z.object({
  type: z.string().optional(),
  impersonate: z.string().optional(),
  proxy: z.string().optional(),
  headless: z.boolean().optional(),
  useragent: z.string().optional(),
  solve_cloudflare: z.boolean().optional(),
  real_chrome: z.boolean().optional(),
})
export type SessionOpenRequest = z.infer<typeof sessionOpenRequestSchema>

export const sessionOpenResponseSchema = z.object({
  session_id: z.string(),
  type: z.string().optional(),
})
export type SessionOpenResponse = z.infer<typeof sessionOpenResponseSchema>

export const sessionFetchRequestSchema = z.object({
  session_id: z.string(),
  url: z.string(),
  method: z.string().optional(),
  selectors: z.array(selectorSpecSchema).optional(),
  format: z.string().optional(),
  include_html: z.boolean().optional(),
})
export type SessionFetchRequest = z.infer<typeof sessionFetchRequestSchema>

export const sessionCloseResponseSchema = z.object({
  closed: z.boolean(),
})
export type SessionCloseResponse = z.infer<typeof sessionCloseResponseSchema>

export const sessionCloseRequestSchema = z.object({
  session_id: z.string(),
})
export type SessionCloseRequest = z.infer<typeof sessionCloseRequestSchema>

export const sessionSummarySchema = z.object({
  session_id: z.string(),
  type: z.string().optional(),
  created_at: z.number().optional(),
  last_used: z.number().optional(),
  idle_s: z.number().optional(),
})
export type SessionSummary = z.infer<typeof sessionSummarySchema>

export const sessionListResponseSchema = z.object({
  sessions: z.array(sessionSummarySchema),
})
export type SessionListResponse = z.infer<typeof sessionListResponseSchema>

export const sessionListRequestSchema = z.object({
  type: z.string().optional(),
})
export type SessionListRequest = z.infer<typeof sessionListRequestSchema>

/* ---------------- crawl ---------------- */

export const crawlRequestSchema = z
  .object({
    start_urls: z.array(z.string().min(1)).optional(),
    url: z.string().min(1).optional(),
    fetcher: z.string().optional(),
    selectors: z.array(selectorSpecSchema).optional(),
    allowed_domains: z.array(z.string()).optional(),
    same_domain: z.boolean().optional(),
    max_pages: z.number().optional(),
    max_depth: z.number().optional(),
    concurrency: z.number().optional(),
    format: z.string().optional(),
    stream_name: z.string().optional(),
  })
  .refine(
    (request) =>
      (request.url?.length ?? 0) > 0 ||
      (request.start_urls?.length ?? 0) > 0,
  )
export type CrawlRequest = z.infer<typeof crawlRequestSchema>

export const crawlItemSchema = z.object({
  url: z.string().optional(),
  status: z.number().nullable().optional(),
  extracted: z.record(z.string(), z.unknown()).optional(),
  content: z.string().optional(),
  error: z.string().optional(),
})
export type CrawlItem = z.infer<typeof crawlItemSchema>

export const crawlResponseSchema = z.object({
  stats: z.object({
    crawled: z.number(),
    items: z.number(),
    errors: z.number(),
    stopped: z.string().optional(),
  }),
  items: z.array(crawlItemSchema).optional(),
  // The worker echoes the caller's stream_name/group_id verbatim, so either
  // may arrive as a number or boolean (e.g. group_id:3) — accept those, not
  // just strings, or the whole card fails to parse and renders nothing.
  stream: z
    .object({
      name: z.union([z.string(), z.number(), z.boolean()]).optional(),
      group_id: z.union([z.string(), z.number(), z.boolean()]).optional(),
    })
    .optional(),
})
export type CrawlResponse = z.infer<typeof crawlResponseSchema>

/* ---------------- helpers ---------------- */

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}

export function safeParseResponse<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(unwrapEnvelope(value))
  return parsed.success ? parsed.data : null
}

export function formatChars(n: number): string {
  if (n < 1000) return `${n} chars`
  return `${(n / 1000).toFixed(1)}k chars`
}
