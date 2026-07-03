import type { FunctionCallMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input,
    output,
    durationMs: 842,
    createdAt: now,
    ...extra,
  }
}

const LIST_HTML =
  '<html><body><h1 class="t">Hacker Books</h1><ul>' +
  '<li><a href="/a">Apple Engineering</a></li>' +
  '<li><a href="/b">Banana Networks</a></li></ul>' +
  '<p>price 42 usd then 99</p></body></html>'

/** 320×200 mock-page PNG (header band, hero block, text lines). */
const SCREENSHOT_PNG =
  'iVBORw0KGgoAAAANSUhEUgAAAUAAAADICAIAAAAWZq/8AAACJElEQVR42u3bsQnAIBCGUccIqRzExbJXCkvXsRfrrBAIhhMevAkOvur403FmYFPJCUDAgIABAYOAAQEDAgYEDAIGBAwIGAQMCBgQMCBgEDAgYEDAgIBBwICAAQGDgAEBAwIGBAwCBgIGPOcANiVgEDAgYEDAIGBAwICAAQGDgAEBAwIGAb/XrwIsImAQsIBBwICAQcACBgE7MQgYEDAIWMAgYEDAIGABg4AFDAIGBAwCFjAIGBAwIGAQsIBBwICAQcACBgELGAQMCBgELGAQMCBgQMAgYAGDgAEBg4AFDAIWMAgYEDAIWMAgYEDAIGABg4AFDAIGBAwCFjAI2JVBwICAQcACBgEDAgYBCxgELGAQMCBgEPC3gIE4BAwCBgQMCBgEDAgYEDAgYBAwIGBAwCBgQMBA7IDv2iAsAQsYAQsYBCxgELCAEbCABYyAvZEAAQMCBgEDAgYEDAIGBAwIGBAwCBgQMGDMAMYMAkbAAgYBCxgELGAELGABI2BvJEDAgIBBwICAAQGDgAEBAwIGBAwCBgQMGDOAMYOAEbCAQcACBgELGAELWMAI2BsJEDAgYBAwIGBAwCBgJwABAwIGBAwCBgQMGDOAMYOAEbCAQcACBgELGAQsYATsjQQIGBAwCBgQMCBgQMAgYEDAgIBBwICAAWMGMGYQMAIWsIARsIBBwAIGAQsYAXsjAQIGBAwCBgQMCBgQMAgYEDAgYBCwK4CAgd89AGHCHu/yHx0AAAAASUVORK5CYII='

/* ---------------- scrapling::fetch ---------------- */

export const scraplingFetchExtract = base(
  'scrapling-fetch-extract',
  'scrapling::fetch',
  {
    url: 'https://example.com/books',
    impersonate: 'chrome',
    selectors: [
      { name: 'title', css: 'h1' },
      { name: 'links', css: 'li a', attr: 'href', all: true },
    ],
  },
  wrapHarness({
    status: 200,
    url: 'https://example.com/books',
    headers: { 'content-type': 'text/html; charset=utf-8' },
    cookies: { sid: 'abc123' },
    encoding: 'utf-8',
    extracted: { title: 'Hacker Books', links: ['/a', '/b'] },
  }),
)

export const scraplingFetchHtml = base(
  'scrapling-fetch-html',
  'scrapling::fetch',
  { url: 'https://example.com/', include_html: true },
  wrapHarness({
    status: 200,
    url: 'https://example.com/',
    headers: { 'content-type': 'text/html; charset=UTF-8' },
    cookies: {},
    encoding: 'utf-8',
    html: LIST_HTML,
  }),
)

export const scraplingFetchMarkdown = base(
  'scrapling-fetch-markdown',
  'scrapling::fetch',
  {
    url: 'https://example.com/article',
    format: 'markdown',
    main_content_only: true,
  },
  wrapHarness({
    status: 200,
    url: 'https://example.com/article',
    headers: { 'content-type': 'text/html; charset=utf-8' },
    cookies: {},
    encoding: 'utf-8',
    format: 'markdown',
    content:
      '# The Headline\n\nFirst paragraph of the article body, already stripped of nav and scripts.\n\n- point one\n- point two',
  }),
)

export const scraplingFetchPlain = base(
  'scrapling-fetch-plain',
  'scrapling::fetch',
  { url: 'https://api.example.com/health', method: 'post' },
  wrapHarness({
    status: 204,
    url: 'https://api.example.com/health',
    headers: {},
    cookies: {},
    encoding: 'utf-8',
  }),
)

export const scraplingFetchBulk = base(
  'scrapling-fetch-bulk',
  'scrapling::fetch',
  {
    urls: [
      'https://example.com/p/1',
      'https://example.com/p/2',
      'https://broken.example.com/p/3',
    ],
    selectors: [{ name: 'title', css: 'h1' }],
  },
  wrapHarness({
    results: [
      {
        status: 200,
        url: 'https://example.com/p/1',
        headers: {},
        cookies: {},
        encoding: 'utf-8',
        extracted: { title: 'Page one' },
      },
      {
        status: 404,
        url: 'https://example.com/p/2',
        headers: {},
        cookies: {},
        encoding: 'utf-8',
        extracted: { title: null },
      },
      {
        url: 'https://broken.example.com/p/3',
        error: 'dns lookup failed for host',
      },
    ],
  }),
)

export const scraplingFetchPending = base(
  'scrapling-fetch-pending',
  'scrapling::fetch',
  {
    url: 'https://api.example.com/orders',
    method: 'post',
    impersonate: 'firefox',
    selectors: [{ name: 'ok', css: '.status' }],
  },
  undefined,
  { pendingApproval: true },
)

export const scraplingFetchRunning = base(
  'scrapling-fetch-running',
  'scrapling::dynamic-fetch',
  { url: 'https://app.example.com/dashboard', wait_selector: '.chart' },
  undefined,
  { running: true },
)

/* ---------------- scrapling::stealthy-fetch / dynamic-fetch ---------------- */

export const scraplingStealthyCloudflare = base(
  'scrapling-stealthy-cloudflare',
  'scrapling::stealthy-fetch',
  {
    url: 'https://protected.example.com/catalog',
    solve_cloudflare: true,
    network_idle: true,
    selectors: [{ name: 'items', css: '.card h2', all: true }],
  },
  wrapHarness({
    status: 200,
    url: 'https://protected.example.com/catalog',
    headers: { 'content-type': 'text/html' },
    cookies: { cf_clearance: '…' },
    encoding: 'utf-8',
    extracted: { items: ['Item one', 'Item two', 'Item three'] },
  }),
)

export const scraplingDynamicXhr = base(
  'scrapling-dynamic-xhr',
  'scrapling::dynamic-fetch',
  {
    url: 'https://app.example.com/feed',
    wait_selector: '.feed-item',
    capture_xhr: '/api/',
    selectors: [{ name: 'first_item', css: '.feed-item h3' }],
  },
  wrapHarness({
    status: 200,
    url: 'https://app.example.com/feed',
    headers: { 'content-type': 'text/html' },
    cookies: {},
    encoding: 'utf-8',
    extracted: { first_item: 'Feed item one' },
    captured_xhr: [
      { url: 'https://app.example.com/api/feed?page=1', status: 200 },
    ],
  }),
)

/* ---------------- scrapling::screenshot ---------------- */

export const scraplingScreenshot = base(
  'scrapling-screenshot',
  'scrapling::screenshot',
  { url: 'https://example.com/', fetcher: 'dynamic', full_page: false },
  wrapHarness({
    image_base64: SCREENSHOT_PNG,
    mime: 'image/png',
    url: 'https://example.com/',
  }),
)

export const scraplingScreenshotPending = base(
  'scrapling-screenshot-pending',
  'scrapling::screenshot',
  { url: 'https://example.com/pricing', fetcher: 'stealthy', full_page: true },
  undefined,
  { pendingApproval: true },
)

/* ---------------- parse-only ops ---------------- */

export const scraplingExtract = base(
  'scrapling-extract',
  'scrapling::extract',
  {
    html: LIST_HTML,
    selectors: [
      { name: 'title', css: 'h1' },
      { name: 'links', css: 'li a', attr: 'href', all: true },
      { name: 'price', regex: 'price (\\d+)' },
    ],
  },
  wrapHarness({
    extracted: { title: 'Hacker Books', links: ['/a', '/b'], price: '42' },
  }),
)

export const scraplingExtractAdaptive = base(
  'scrapling-extract-adaptive',
  'scrapling::extract',
  {
    html: LIST_HTML,
    selectors: [{ name: 'title', css: 'h1' }],
    adaptive: true,
    adaptive_domain: 'https://books.example.com',
  },
  wrapHarness({ extracted: { title: 'Hacker Books' } }),
)

export const scraplingCssAll = base(
  'scrapling-css-all',
  'scrapling::css',
  { html: LIST_HTML, query: 'li a' },
  wrapHarness({ result: ['Apple Engineering', 'Banana Networks'] }),
)

export const scraplingCssFirstAttr = base(
  'scrapling-css-first-attr',
  'scrapling::css',
  { html: LIST_HTML, query: 'li a', first: true, attr: 'href' },
  wrapHarness({ result: '/a' }),
)

export const scraplingXpathFirst = base(
  'scrapling-xpath-first',
  'scrapling::xpath',
  { html: LIST_HTML, query: '//h1', first: true },
  wrapHarness({ result: 'Hacker Books' }),
)

export const scraplingRegexAll = base(
  'scrapling-regex-all',
  'scrapling::regex',
  { html: LIST_HTML, pattern: '\\d+' },
  wrapHarness({ result: ['42', '99'] }),
)

export const scraplingRegexNoMatch = base(
  'scrapling-regex-no-match',
  'scrapling::regex',
  { html: LIST_HTML, pattern: 'sku-[A-Z]+', first: true },
  wrapHarness({ result: null }),
)

export const scraplingFindSimilar = base(
  'scrapling-find-similar',
  'scrapling::find-similar',
  {
    html: LIST_HTML,
    anchor: 'li',
    selectors: [{ name: 'href', css: 'a', attr: 'href' }],
  },
  wrapHarness({ count: 2, items: [{ href: '/a' }, { href: '/b' }] }),
)

/* ---------------- element search (Phase 1) ---------------- */

const LINK_A = {
  tag: 'a',
  text: 'Apple Engineering',
  html: '<a href="/a">Apple Engineering</a>',
  attrs: { href: '/a' },
  css: 'body > ul > li > a',
  xpath: '//body/ul/li/a',
}
const LINK_B = {
  tag: 'a',
  text: 'Banana Networks',
  html: '<a href="/b">Banana Networks</a>',
  attrs: { href: '/b' },
  css: 'body > ul > li:nth-of-type(2) > a',
  xpath: '//body/ul/li[2]/a',
}

export const scraplingFind = base(
  'scrapling-find',
  'scrapling::find',
  { html: LIST_HTML, tag: 'a', attrs: { href: '/a' } },
  wrapHarness({ count: 1, items: [LINK_A] }),
)

export const scraplingFindByText = base(
  'scrapling-find-by-text',
  'scrapling::find-by-text',
  { html: LIST_HTML, text: 'Engineering', partial: true },
  wrapHarness({ count: 1, items: [LINK_A] }),
)

export const scraplingFindByRegex = base(
  'scrapling-find-by-regex',
  'scrapling::find-by-regex',
  { html: LIST_HTML, pattern: '(Apple|Banana)' },
  wrapHarness({ count: 2, items: [LINK_A, LINK_B] }),
)

export const scraplingFindEmpty = base(
  'scrapling-find-empty',
  'scrapling::find-by-text',
  { html: LIST_HTML, text: 'Zebra' },
  wrapHarness({ count: 0, items: [] }),
)

export const scraplingDescribe = base(
  'scrapling-describe',
  'scrapling::describe',
  { html: LIST_HTML, query: 'h1' },
  wrapHarness({
    found: true,
    element: {
      tag: 'h1',
      text: 'Hacker Books',
      html: '<h1 class="t">Hacker Books</h1>',
      attrs: { class: 't' },
      css: 'body > h1',
      xpath: '//body/h1',
      full_css: 'html > body > h1.t',
      full_xpath: '/html/body/h1',
      classes: ['t'],
      parent_tag: 'body',
      children: 0,
      siblings: 1,
    },
  }),
)

export const scraplingDescribeMissing = base(
  'scrapling-describe-missing',
  'scrapling::describe',
  { html: LIST_HTML, query: '.nope' },
  wrapHarness({ found: false }),
)

export const scraplingToMarkdown = base(
  'scrapling-to-markdown',
  'scrapling::to-markdown',
  { html: LIST_HTML, format: 'markdown', main_content_only: true },
  wrapHarness({
    format: 'markdown',
    content:
      '# Hacker Books\n\n- [Apple Engineering](/a)\n- [Banana Networks](/b)\n\nprice 42 usd then 99',
  }),
)

/* ---------------- sessions (Phase 4) ---------------- */

const SID = 'a1b2c3d4e5f6a1b2c3d4e5f6'

export const scraplingSessionOpen = base(
  'scrapling-session-open',
  'scrapling::session-open',
  { type: 'stealthy', solve_cloudflare: true },
  wrapHarness({ session_id: SID, type: 'stealthy' }),
)

export const scraplingSessionOpenPending = base(
  'scrapling-session-open-pending',
  'scrapling::session-open',
  { type: 'dynamic', real_chrome: true },
  undefined,
  { pendingApproval: true },
)

export const scraplingSessionFetch = base(
  'scrapling-session-fetch',
  'scrapling::session-fetch',
  {
    session_id: SID,
    url: 'https://app.example.com/account',
    selectors: [{ name: 'user', css: '.username' }],
  },
  wrapHarness({
    status: 200,
    url: 'https://app.example.com/account',
    headers: { 'content-type': 'text/html' },
    cookies: { session: 'kept' },
    encoding: 'utf-8',
    extracted: { user: 'ada@example.com' },
  }),
)

export const scraplingSessionList = base(
  'scrapling-session-list',
  'scrapling::session-list',
  {},
  wrapHarness({
    sessions: [
      {
        session_id: SID,
        type: 'stealthy',
        created_at: 1,
        last_used: 2,
        idle_s: 4.2,
      },
      {
        session_id: 'ffffeeee0000',
        type: 'http',
        created_at: 1,
        last_used: 2,
        idle_s: 88,
      },
    ],
  }),
)

export const scraplingSessionClose = base(
  'scrapling-session-close',
  'scrapling::session-close',
  { session_id: SID },
  wrapHarness({ closed: true }),
)

/* ---------------- crawl (Phase 5) ---------------- */

export const scraplingCrawl = base(
  'scrapling-crawl',
  'scrapling::crawl',
  {
    start_urls: ['https://blog.example.com/'],
    fetcher: 'http',
    selectors: [{ name: 'title', css: 'h1' }],
    max_pages: 20,
    max_depth: 2,
  },
  wrapHarness({
    stats: { crawled: 12, items: 11, errors: 1, stopped: 'done' },
    items: [
      {
        url: 'https://blog.example.com/',
        status: 200,
        extracted: { title: 'Blog Home' },
      },
      {
        url: 'https://blog.example.com/post/1',
        status: 200,
        extracted: { title: 'First Post' },
      },
      {
        url: 'https://blog.example.com/post/2',
        status: 200,
        extracted: { title: 'Second Post' },
      },
      {
        url: 'https://blog.example.com/gone',
        error: 'dns lookup failed for host',
      },
    ],
    stream: { name: 'scrapling::crawl', group_id: 'c0ffee1234' },
  }),
)

export const scraplingCrawlPending = base(
  'scrapling-crawl-pending',
  'scrapling::crawl',
  {
    start_urls: ['https://shop.example.com/', 'https://shop.example.com/sale'],
    fetcher: 'stealthy',
    max_pages: 50,
    max_depth: 3,
    selectors: [{ name: 'price', css: '.price' }],
  },
  undefined,
  { pendingApproval: true },
)

export const scraplingCrawlCapped = base(
  'scrapling-crawl-capped',
  'scrapling::crawl',
  { start_urls: ['https://big.example.com/'], max_pages: 5 },
  wrapHarness({
    stats: { crawled: 5, items: 5, errors: 0, stopped: 'max_pages' },
    items: [{ url: 'https://big.example.com/', status: 200 }],
    stream: { name: 'scrapling::crawl', group_id: 'abcd00' },
  }),
)

/* ---------------- errors ---------------- */

export const scraplingFetchDenied = base(
  'scrapling-fetch-denied',
  'scrapling::stealthy-fetch',
  { url: 'https://protected.example.com/catalog', solve_cloudflare: true },
  {
    error: {
      kind: 'function_error',
      message: 'trigger_failed: denied',
      details: {
        status: 'denied',
        denied_by: 'user',
        function_id: 'scrapling::stealthy-fetch',
        reason: 'User denied the browser fetch.',
      },
      content: [{ type: 'text', text: 'User denied the browser fetch.' }],
    },
  },
)

export const scraplingFetchHandlerError = base(
  'scrapling-fetch-handler-error',
  'scrapling::fetch',
  {},
  {
    error: {
      kind: 'function_error',
      message: 'trigger_failed: provide `url` or `urls`',
      content: [{ type: 'text', text: 'provide `url` or `urls`' }],
    },
  },
)

export const scraplingFixtures = [
  scraplingFetchExtract,
  scraplingFetchHtml,
  scraplingFetchMarkdown,
  scraplingFetchPlain,
  scraplingFetchBulk,
  scraplingFetchPending,
  scraplingFetchRunning,
  scraplingStealthyCloudflare,
  scraplingDynamicXhr,
  scraplingScreenshot,
  scraplingScreenshotPending,
  scraplingExtract,
  scraplingExtractAdaptive,
  scraplingCssAll,
  scraplingCssFirstAttr,
  scraplingXpathFirst,
  scraplingRegexAll,
  scraplingRegexNoMatch,
  scraplingFindSimilar,
  scraplingSessionOpen,
  scraplingSessionOpenPending,
  scraplingSessionFetch,
  scraplingSessionList,
  scraplingSessionClose,
  scraplingCrawl,
  scraplingCrawlPending,
  scraplingCrawlCapped,
  scraplingFind,
  scraplingFindByText,
  scraplingFindByRegex,
  scraplingFindEmpty,
  scraplingDescribe,
  scraplingDescribeMissing,
  scraplingToMarkdown,
  scraplingFetchDenied,
  scraplingFetchHandlerError,
] as const
