/**
 * browser has no database-style drivers or dialect matrix. Each case
 * is a direct bus call, so there's no `CaseContext.driver` /
 * `dialect` (contrast database's harness). Expected values below are NOT
 * invented: each case is either a byte-for-byte copy of a request/response
 * pair from ../../../../golden/behavior/**\/*.json (the differential fixtures
 * the Rust `tests/behavior.rs` test replays against the reference Python
 * implementation), or — where noted — a small, code-verified extension of
 * one (e.g. adding a redundant `tag` alongside an already-golden `attrs`
 * filter that provably selects the same element set; see the comment on
 * that case).
 */

export interface CaseContext {
  /** Calls a browser function; returns parsed JSON or throws on engine error. */
  call: (functionId: string, payload: unknown) => Promise<any>
  /** Hermetic loopback origin owned by the harness for outbound cases. */
  origin: string
}

export interface TestCase {
  name: string
  run(ctx: CaseContext): Promise<void>
}

/** Recursively sorts object keys so JSON.stringify comparison is order-insensitive
 * (array element order still matters). browser's Cargo.toml enables
 * serde_json's `preserve_order`, so the wire key order tracks *insertion*
 * order in the Rust source — a cosmetic future reorder of a `json!({...})`
 * literal shouldn't fail this suite. Same technique database's own
 * cases-boundary.ts uses for its JSONB round-trip case (`canon`). */
function canon(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(canon)
  if (v !== null && typeof v === 'object') {
    const out: Record<string, unknown> = {}
    for (const k of Object.keys(v as Record<string, unknown>).sort()) {
      out[k] = canon((v as Record<string, unknown>)[k])
    }
    return out
  }
  return v
}

export function expectEqual(actual: unknown, expected: unknown, msg: string): void {
  if (JSON.stringify(canon(actual)) !== JSON.stringify(canon(expected))) {
    throw new Error(`${msg}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

export function expect(cond: boolean, msg: string): asserts cond {
  if (!cond) throw new Error(msg)
}

/**
 * Asserts the complete error visible through the iii bus. Partial matches hide
 * wrapper regressions, especially when two errors share a prefix.
 */
export async function expectError(
  fn: () => Promise<unknown>,
  expected: string,
  label: string,
): Promise<void> {
  try {
    await fn()
  } catch (e: any) {
    const msg = e?.message ?? String(e)
    if (msg !== expected) {
      throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(msg)}`)
    }
    return
  }
  throw new Error(`${label}: expected the call to reject, but it resolved`)
}

// Shared fixture HTML, copied verbatim (including the trailing newline) from
// tests/golden/behavior/{css,xpath,extract,regex,find,find-by-text,
// find-by-regex,find-similar,describe,to-markdown}/*.json — the same string
// backs 8 of the 10 happy-path cases below, exactly as it does across those
// golden fixtures.
const BASIC_HTML =
  '<html><head></head><body><h1 class="t">Hello</h1><ul><li><a href="/a">Apple</a></li><li><a href="/b">Banana</a></li></ul><p>price 42 usd then 99</p></body></html>\n'

// Copied verbatim from tests/golden/behavior/find/limit_clamps.json (also
// backs find-by-regex/messy_cards.json and find/attrs_exact_whole_value.json).
const MESSY_HTML =
  '<html><head><title>Messy Page</title><meta charset="utf-8"></head><body>\n<nav><ul><li><a href="/home">Home</a></li><li><a href="/about">About</a></li></ul></nav>\n<template><p>never render this</p></template>\n<div aria-hidden="true">screen-reader trap</div>\n<div style="display:none">hidden A</div>\n<div style="visibility: hidden">hidden B</div>\n<main>\n  <h1>Widget Review</h1>\n  <p>Intro paragraph with <em>emphasis</em> and a <a href="/w/1">link</a>.</p>\n  <h2>Specs</h2>\n  <table><tr><th>Name</th><th>Value</th></tr><tr><td>Weight</td><td>3kg</td></tr></table>\n  <ul><li>alpha<ul><li>nested</li></ul></li><li>beta</li></ul>\n  <div class="card" data-id="1"><h3>Card One</h3><p>first card</p></div>\n  <div class="card" data-id="2"><h3>Card Two</h3><p>second card</p></div>\n  <div class="card wide" data-id="3"><h3>Card Three</h3><p>third card</p></div>\n</main>\n<footer><p>© 2026 Example — <span style="font-size:0">invisible</span>fine print</p></footer>\n<script>analytics()</script>\n</body></html>\n'

// Copied verbatim from the `const HTML` in src/xpath/eval.rs's test module
// (the fixture behind its `error_shapes` test, which asserts the exact
// "//p/ancestor::div" input this case reuses).
const ANCESTOR_HTML = '<main id="m"><section id="s"><p>P</p></section></main>'

export const ORIGIN_PAGE_HTML =
  '<!doctype html><html><head><title>E2E</title></head><body><h1>initial</h1><div id="fp"></div><a href="/leaf">leaf</a><script>document.querySelector("h1").textContent="rendered";document.querySelector("#fp").textContent=[navigator.language,Intl.DateTimeFormat().resolvedOptions().timeZone,devicePixelRatio,innerWidth,innerHeight].join("|")</script></body></html>'

export const CASES: TestCase[] = [
  {
    // Golden: tests/golden/behavior/extract/mixed_specs.json
    name: 'extract: mixed selector list (css/xpath precedence, regex, attr, html, all)',
    async run({ call }) {
      const r = await call('browser::extract', {
        html: BASIC_HTML,
        selectors: [
          { name: 'title', css: 'h1' },
          { name: 'links', css: 'li a', attr: 'href', all: true },
          { name: 'names', css: 'li a', all: true },
          { name: 'price', regex: 'price (\\d+)' },
          { name: 'first_li_html', css: 'li', html: true },
        ],
      })
      expectEqual(
        r,
        {
          extracted: {
            title: 'Hello',
            links: ['/a', '/b'],
            names: ['Apple', 'Banana'],
            price: '42',
            first_li_html: '<li><a href="/a">Apple</a></li>',
          },
        },
        'extract mixed_specs',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/css/first_attr.json
    name: 'css: first + attr pulls the attribute of the first match',
    async run({ call }) {
      const r = await call('browser::css', { html: BASIC_HTML, query: 'li a', first: true, attr: 'href' })
      expectEqual(r, { result: '/a' }, 'css first_attr')
    },
  },
  {
    // Golden: tests/golden/behavior/xpath/positional.json
    name: 'xpath: positional predicate + child step, first (text)',
    async run({ call }) {
      const r = await call('browser::xpath', { html: BASIC_HTML, query: '//li[2]/a', first: true })
      expectEqual(r, { result: 'Banana' }, 'xpath positional')
    },
  },
  {
    // Golden: tests/golden/behavior/regex/first_group.json
    name: 'regex: first capture group',
    async run({ call }) {
      const r = await call('browser::regex', { html: BASIC_HTML, pattern: 'price (\\d+)', first: true })
      expectEqual(r, { result: '42' }, 'regex first_group')
    },
  },
  {
    // Golden: tests/golden/behavior/find/attrs_exact_whole_value.json, on
    // MESSY_HTML. That fixture filters by `attrs` alone (base selector
    // `*[class="card"]`, per src/functions/find.rs's `op`: an empty `tag`
    // falls back to `base = ["*"]` before the attrs suffix is appended).
    // Adding `tag: "div"` narrows the base to `div[class="card"]` instead —
    // provably the same match set here, since every `class="card"` element
    // in this fixture already *is* a <div> (there is no other tag carrying
    // that class) — so this exercises the tag+attrs combination the golden
    // itself doesn't, without inventing an unverified result: same 2 items,
    // same count.
    name: 'find: tag + attrs (count/items)',
    async run({ call }) {
      const r = await call('browser::find', { html: MESSY_HTML, tag: 'div', attrs: { class: 'card' } })
      expectEqual(
        r,
        {
          count: 2,
          items: [
            {
              tag: 'div',
              text: 'Card One\nfirst card',
              html: '<div class="card" data-id="1"><h3>Card One</h3><p>first card</p></div>',
              attrs: { class: 'card', 'data-id': '1' },
              css: 'body > main > div',
              xpath: '//body/main/div',
            },
            {
              tag: 'div',
              text: 'Card Two\nsecond card',
              html: '<div class="card" data-id="2"><h3>Card Two</h3><p>second card</p></div>',
              attrs: { class: 'card', 'data-id': '2' },
              css: 'body > main > div:nth-of-type(2)',
              xpath: '//body/main/div[2]',
            },
          ],
        },
        'find tag+attrs',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/find-by-text/exact_default.json
    name: 'find-by-text: leading-text exact match',
    async run({ call }) {
      const r = await call('browser::find-by-text', { html: BASIC_HTML, text: 'Apple' })
      expectEqual(
        r,
        {
          count: 1,
          items: [
            {
              tag: 'a',
              text: 'Apple',
              html: '<a href="/a">Apple</a>',
              attrs: { href: '/a' },
              css: 'body > ul > li > a',
              xpath: '//body/ul/li/a',
            },
          ],
        },
        'find-by-text exact_default',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/find-by-regex/default_insensitive.json
    name: 'find-by-regex: leading-text pattern match (case-insensitive default)',
    async run({ call }) {
      const r = await call('browser::find-by-regex', { html: BASIC_HTML, pattern: 'price \\d+' })
      expectEqual(
        r,
        {
          count: 1,
          items: [
            {
              tag: 'p',
              text: 'price 42 usd then 99',
              html: '<p>price 42 usd then 99</p>',
              attrs: {},
              css: 'body > p',
              xpath: '//body/p',
            },
          ],
        },
        'find-by-regex default_insensitive',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/find-similar/list_items.json
    name: 'find-similar: li anchor (count 2)',
    async run({ call }) {
      const r = await call('browser::find-similar', { html: BASIC_HTML, anchor: 'li' })
      expectEqual(
        r,
        {
          count: 2,
          items: [
            { text: 'Apple', html: '<li><a href="/a">Apple</a></li>' },
            { text: 'Banana', html: '<li><a href="/b">Banana</a></li>' },
          ],
        },
        'find-similar list_items',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/describe/h1_css.json
    name: 'describe: h1 (found, classes, parent_tag)',
    async run({ call }) {
      const r = await call('browser::describe', { html: BASIC_HTML, query: 'h1' })
      expectEqual(
        r,
        {
          found: true,
          element: {
            tag: 'h1',
            text: 'Hello',
            html: '<h1 class="t">Hello</h1>',
            attrs: { class: 't' },
            css: 'body > h1',
            xpath: '//body/h1',
            full_css: 'body > h1',
            full_xpath: '//body/h1',
            classes: ['t'],
            parent_tag: 'body',
            children: 0,
            siblings: 2,
          },
        },
        'describe h1_css',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/to-markdown/text_basic.json — deterministic
    // (no HTML->Markdown formatting choices in text mode), so an exact string.
    name: 'to-markdown: text mode (exact string)',
    async run({ call }) {
      const r = await call('browser::to-markdown', { html: BASIC_HTML, format: 'text' })
      expectEqual(r, { format: 'text', content: 'Hello\nApple\nBanana\nprice 42 usd then 99' }, 'to-markdown text_basic')
    },
  },
  {
    name: 'css: adaptive direct match',
    async run({ call }) {
      expectEqual(
        await call('browser::css', {
          html: '<p>x</p>',
          query: 'p',
          adaptive: true,
          auto_save: false,
          adaptive_domain: 'e2e.invalid',
          identifier: 'e2e-direct',
        }),
        { result: ['x'] },
        'adaptive direct match',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/css/invalid_selector.json, plus the exact
    // iii bus error envelope observed by callers.
    name: 'css: invalid selector error',
    async run({ call }) {
      await expectError(
        () => call('browser::css', { html: BASIC_HTML, query: 'li:::bad' }),
        "handler error: Invalid CSS selector 'li:::bad': Expected ident, got <DELIM ':' at 4>",
        'invalid css selector',
      )
    },
  },
  {
    // Golden: xpath/ancestor_axis_reverse_position.json.
    name: 'xpath: ancestor axis reverse position',
    async run({ call }) {
      expectEqual(
        await call('browser::xpath', { html: ANCESTOR_HTML, query: '//p/ancestor::*[1]/@id' }),
        { result: ['s'] },
        'ancestor axis',
      )
    },
  },
  {
    // Golden: tests/golden/behavior/find/negative_limit_empty.json uses
    // limit=-1 on this exact html+tag ("a" on BASIC_HTML — same fixture
    // find/by_tag.json proves has 2 matches with no limit) and gets
    // count=2, items=[]. src/functions/common.rs::bounded clamps limit to
    // `[0, MAX_FIND_ITEMS]` — `l.clamp(0, MAX)` maps BOTH -1 and 0 to the
    // same ceiling of 0 — so limit=0 is the same code path the golden
    // already covers, just via the other input that clamps to it.
    name: 'find: limit 0 clamps items to [], count stays the true total',
    async run({ call }) {
      const r = await call('browser::find', { html: BASIC_HTML, tag: 'a', limit: 0 })
      expectEqual(r, { count: 2, items: [] }, 'find limit_clamp')
    },
  },

  // ---- outbound surface -------------------------------------------------
  // Every successful outbound case uses the harness-owned loopback origin.
  // External egress remains unnecessary and SSRF policy is still exercised.
  {
    // The SSRF guard is the reason these functions can take an arbitrary
    // caller-supplied URL at all, so it gets an end-to-end assertion, not
    // just a unit test: 169.254.169.254 is the cloud metadata endpoint.
    name: 'fetch: SSRF guard refuses the link-local metadata address',
    async run({ call }) {
      await expectError(
        () => call('browser::fetch', { url: 'http://169.254.169.254/latest/meta-data/' }),
        'handler error: address 169.254.169.254 is in link-local (incl. AWS metadata)',
        'ssrf link-local',
      )
    },
  },
  {
    name: 'fetch: non-http scheme refused',
    async run({ call }) {
      await expectError(
        () => call('browser::fetch', { url: 'file:///etc/passwd' }),
        'handler error: scheme not allowed: file: (only http: and https: are permitted)',
        'ssrf scheme',
      )
    },
  },
  {
    name: 'fetch: unsupported method named in the error',
    async run({ call }) {
      await expectError(
        () => call('browser::fetch', { url: 'https://example.com/', method: 'patch' }),
        'handler error: unsupported method: patch',
        'fetch method',
      )
    },
  },
  {
    name: 'fetch: neither url nor urls',
    async run({ call }) {
      await expectError(
        () => call('browser::fetch', {}),
        'handler error: provide `url` or `urls`',
        'fetch no target',
      )
    },
  },
  {
    name: 'browser fetchers: dynamic and stealthy render the local page',
    async run({ call, origin }) {
      for (const [functionId, viewport] of [
        ['browser::dynamic-fetch', '1280|720'],
        ['browser::stealthy-fetch', '1920|1080'],
      ] as const) {
        const result = await call(functionId, {
          url: `${origin}/page`,
          include_html: true,
          locale: 'fr-FR',
          timezone_id: 'Europe/Paris',
          retries: 1,
          timeout: 5000,
          solve_cloudflare: functionId === 'browser::stealthy-fetch',
        })
        expectEqual(result.status, 200, `${functionId} status`)
        expectEqual(result.url, `${origin}/page`, `${functionId} url`)
        expectEqual(result.cookies, {}, `${functionId} cookie quirk`)
        expectEqual(result.encoding, 'utf-8', `${functionId} encoding`)
        expect(result.html.includes('<h1>rendered</h1>'), `${functionId} did not execute page script`)
        expect(
          result.html.includes(`fr-FR|Europe/Paris|2|${viewport}`),
          `${functionId} fingerprint did not match the frozen viewport`,
        )
      }
    },
  },
  {
    name: 'screenshot-url: unknown fetcher preserves the dynamic fallback quirk',
    async run({ call, origin }) {
      const result = await call('browser::screenshot-url', {
        url: `${origin}/page`,
        fetcher: 'telepathy',
        format: 'png',
        retries: 1,
        timeout: 5000,
      })
      expectEqual(result.mime, 'image/png', 'screenshot mime')
      expectEqual(result.url, `${origin}/page`, 'screenshot url')
      expectEqual(result.content.length, 2, 'screenshot content blocks')
      expectEqual(result.content[0].type, 'image', 'screenshot image block')
      expectEqual(result.content[0].mime, 'image/png', 'screenshot image block mime')
      expect(
        Buffer.from(result.content[0].data, 'base64').subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])),
        'screenshot is not a PNG',
      )
      expectEqual(result.content[1].type, 'text', 'screenshot caption block')
      expect(
        result.content[1].text.startsWith(`screenshot of ${origin}/page — 1024x576px, 1 tile(s), `),
        `unexpected screenshot caption: ${result.content[1].text}`,
      )
    },
  },
  {
    name: 'crawl: missing start_urls',
    async run({ call }) {
      await expectError(
        () => call('browser::crawl', {}),
        'handler error: provide `start_urls`',
        'crawl no seeds',
      )
    },
  },
  {
    name: 'session: HTTP cookie state, list order, and idempotent close',
    async run({ call, origin }) {
      const opened = await call('browser::session-open', { type: 'http' })
      if (typeof opened.session_id !== 'string' || !/^[0-9a-f]{32}$/.test(opened.session_id)) {
        throw new Error(`expected a UUID4 hex http session id, got ${JSON.stringify(opened)}`)
      }
      expectEqual(opened.type, 'http', 'session-open type')

      const listed = await call('browser::session-list', { type: 'http' })
      const mine = (listed.sessions ?? []).find((s: any) => s.session_id === opened.session_id)
      if (!mine) throw new Error(`session-list omitted ${opened.session_id}: ${JSON.stringify(listed)}`)
      for (const k of ['session_id', 'type', 'created_at', 'last_used', 'idle_s']) {
        if (!(k in mine)) throw new Error(`session-list entry missing '${k}': ${JSON.stringify(mine)}`)
      }

      const first = await call('browser::session-fetch', {
        session_id: opened.session_id,
        url: `${origin}/cookie`,
        include_html: true,
        retries: 1,
      })
      const second = await call('browser::session-fetch', {
        session_id: opened.session_id,
        url: `${origin}/cookie`,
        include_html: true,
        retries: 1,
      })
      expect(first.html.includes('<p>none</p>'), `first HTTP session fetch unexpectedly had a cookie: ${first.html}`)
      expect(second.html.includes('<p>sid=abc</p>'), `HTTP session did not retain its cookie: ${second.html}`)

      expectEqual(
        await call('browser::session-close', { session_id: opened.session_id }),
        { closed: true },
        'session-close first',
      )
      expectEqual(
        await call('browser::session-close', { session_id: opened.session_id }),
        { closed: false },
        'session-close idempotent',
      )
    },
  },
  {
    // Safe mode refuses caller proxies before dialing: their own DNS/routing
    // could bypass the address pinned by the native egress policy.
    name: 'fetch: safe mode refuses a caller proxy',
    async run({ call }) {
      await expectError(
        () =>
          call('browser::fetch', {
            url: 'https://example.com/',
            proxy: 'http://169.254.169.254:3128',
          }),
        'handler error: safe mode refuses `proxy`: a caller proxy can resolve or route to addresses outside the egress policy; use a certified compat build or remove the option',
        'safe proxy policy',
      )
    },
  },
  {
    name: 'fetch: safe HTTP happy path returns the frozen envelope',
    async run({ call, origin }) {
      const result = await call('browser::fetch', {
        url: `${origin}/page`,
        include_html: true,
        timeout: 1e20,
        retries: 1,
      })
      expectEqual(result.status, 200, 'HTTP status')
      expectEqual(result.url, `${origin}/page`, 'HTTP url')
      expectEqual(result.cookies, { sid: 'abc' }, 'HTTP cookies')
      expectEqual(result.encoding, 'utf-8', 'HTTP encoding')
      expectEqual(result.html, ORIGIN_PAGE_HTML.replace('<!doctype html>', ''), 'HTTP normalized HTML')
    },
  },
  {
    name: 'crawl: local HTTP completion, sample, and one-based stream metadata',
    async run({ call, origin }) {
      const result = await call('browser::crawl', {
        url: `${origin}/page`,
        fetcher: 'http',
        max_pages: 2,
        max_depth: 1,
        concurrency: 1,
        group_id: 'e2e-crawl',
        selectors: [{ name: 'heading', css: 'h1' }],
      })
      expectEqual(
        result,
        {
          stats: { crawled: 2, items: 2, errors: 0, stopped: 'done' },
          items: [
            { url: `${origin}/page`, status: 200, extracted: { heading: 'initial' } },
            { url: `${origin}/leaf`, status: 200, extracted: { heading: null } },
          ],
          stream: { name: 'browser::crawl', group_id: 'e2e-crawl' },
        },
        'crawl result',
      )
    },
  },
  {
    name: 'session: dynamic and stealthy browser backends fetch and close',
    async run({ call, origin }) {
      for (const type of ['dynamic', 'stealthy'] as const) {
        const opened = await call('browser::session-open', {
          type,
          locale: 'fr-FR',
          timezone_id: 'Europe/Paris',
          solve_cloudflare: type === 'stealthy',
        })
        expectEqual(opened.type, type, `${type} session type`)
        expect(/^[0-9a-f]{32}$/.test(opened.session_id), `${type} session id is not UUID4 hex`)
        const fetched = await call('browser::session-fetch', {
          session_id: opened.session_id,
          url: `${origin}/page`,
          include_html: true,
          retries: 1,
          timeout: 5000,
        })
        expectEqual(fetched.status, 200, `${type} session status`)
        expect(fetched.html.includes('<h1>rendered</h1>'), `${type} session did not render`)
        expectEqual(await call('browser::session-close', { session_id: opened.session_id }), { closed: true }, `${type} close`)
      }
    },
  },
  {
    name: 'session-fetch: unknown session id',
    async run({ call }) {
      await expectError(
        () => call('browser::session-fetch', { session_id: 'b999', url: 'https://example.com/' }),
        'handler error: unknown session: b999',
        'session unknown',
      )
    },
  },
]
