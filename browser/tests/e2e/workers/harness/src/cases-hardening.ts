/**
 * E2E coverage for the scrapling-native hardening pass: one case per feature
 * that is observable over the iii bus. Expected error strings are pinned
 * full-length against the live worker (expectError is full-string equality).
 *
 * Deliberately NOT covered here — no wire-observable behavior exists, each is
 * certified by unit tests in the worker crate instead:
 *  - chromiumoxide log filter (src/logging.rs): needs protocol-skew frames the
 *    frozen CI Chromium may never emit
 *  - CDP per-command 180s ceiling and Lagged-recoverable/broadcast sizing
 *    (src/scrapling/cdp.rs): internal, and 180s exceeds the suite budget
 *  - retry-delay 60s cap timing (raw_browser.rs): only a timing side channel
 *  - target-close-on-error leak fixes (raw_browser.rs): no external counter
 *  - sessions insertion_order pruning (sessions.rs): filtered out of
 *    session-list; the capacity path it protects is covered below
 *  - OOPIF iframe HTML: main-frame-only serialization by design, and same-host
 *    ports are same-site so no OOPIF forms locally; the route_child_targets
 *    fix is covered via the dedicated-Web-Worker case instead
 */

import { expect, expectEqual, expectError, type CaseContext, type TestCase } from './cases.ts'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

type Call = CaseContext['call']

async function getBrowserConfig(call: Call): Promise<any> {
  const got = await call('configuration::get', { id: 'browser' })
  return got.value
}

async function setBrowserConfig(call: Call, value: unknown): Promise<void> {
  await call('configuration::set', { id: 'browser', value })
}

async function guidanceTriggerVisible(call: Call): Promise<boolean> {
  const listed = await call('engine::registered-triggers::list', { include_internal: true })
  return (listed.registered_triggers ?? []).some(
    (t: any) =>
      t.trigger_type === 'harness::hook::pre-generate' && t.function_id === 'browser::inject-guidance',
  )
}

async function pollUntil(cond: () => Promise<boolean>, label: string, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await cond()) return
    await sleep(200)
  }
  throw new Error(`timed out waiting for ${label}`)
}

// Five matches for the limit-coercion case; count must stay the pre-cap total.
const LIMIT_HTML = '<html><body><ul><li>a</li><li>b</li><li>c</li><li>d</li><li>e</li></ul></body></html>'

export const HARDENING_CASES: TestCase[] = [
  {
    // NAT64/6to4/IPv4-compatible IPv6 literals embed a v4 address that must
    // run the v4 blocklist. Loopback embeds are NOT used here: this stack
    // sets allow_loopback=true, which legitimately admits them.
    name: 'ssrf: v6-embedded-v4 forms run the v4 blocklist',
    async run({ call }) {
      await expectError(
        () => call('browser::fetch', { url: 'http://[64:ff9b::a9fe:a9fe]/' }),
        'handler error: address 64:ff9b::a9fe:a9fe is in link-local (incl. AWS metadata)',
        'nat64 well-known prefix embedding the metadata address',
      )
      await expectError(
        () => call('browser::fetch', { url: 'http://[64:ff9b:1::1]/' }),
        'handler error: address 64:ff9b:1::1 is in local-use nat64 64:ff9b:1::/48',
        'local-use nat64 prefix is refused wholesale',
      )
      await expectError(
        () => call('browser::fetch', { url: 'http://[2002:a00:1::]/' }),
        'handler error: address 2002:a00:1:: is in private rfc1918',
        '6to4 prefix embedding an rfc1918 address',
      )
    },
  },
  {
    name: 'egress gate: forces Connection: close and denies blocked addresses with a 403 page',
    async run({ call, origin }) {
      const echoed = await call('browser::dynamic-fetch', {
        url: `${origin}/echo-headers`,
        include_html: true,
        retries: 1,
        timeout: 8000,
      })
      expectEqual(echoed.status, 200, 'echo-headers status')
      const connections = echoed.html.match(/"connection"/gi) ?? []
      expectEqual(connections.length, 1, 'exactly one Connection header reaches the origin')
      expect(/"connection","close"/i.test(echoed.html), 'the forwarded Connection header is close')
      expect(!/proxy-connection/i.test(echoed.html), 'Proxy-Connection must be stripped')
      expect(!/keep-alive/i.test(echoed.html), 'keep-alive must be stripped')
      expect(
        echoed.html.includes('"url":"/echo-headers"'),
        'the gate re-emits the absolute-form proxy request in origin-form',
      )

      // The gate is the whole SSRF defense for browser tiers: a blocked
      // address is not a worker error but a 403 page Chromium renders.
      const denied = await call('browser::dynamic-fetch', {
        url: 'http://169.254.169.254/',
        include_html: true,
        retries: 1,
        timeout: 8000,
      })
      expectEqual(denied.status, 403, 'gate denial status')
      expect(
        denied.html.includes(
          'browser egress denied: address 169.254.169.254 is in link-local (incl. AWS metadata)',
        ),
        `gate denial body missing: ${denied.html}`,
      )
    },
  },
  {
    name: 'fetch: max_redirects refuses -1 in safe mode and clamps large values to 100',
    async run({ call, origin }) {
      await expectError(
        () => call('browser::fetch', { url: `${origin}/loop`, max_redirects: -1 }),
        'handler error: safe mode refuses `max_redirects:-1`: unlimited or invalid redirect counts exceed the bounded request policy; use a non-negative limit',
        'unlimited redirects refused',
      )
      await expectError(
        () => call('browser::fetch', { url: `${origin}/loop`, max_redirects: 5000, retries: 1 }),
        `handler error: too many redirects (>100) starting at ${origin}/loop`,
        'redirect budget clamps to 100, not 5000',
      )
      const halted = await call('browser::fetch', { url: `${origin}/loop`, follow_redirects: false, retries: 1 })
      expectEqual(halted.status, 302, 'follow_redirects:false returns the redirect itself')
    },
  },
  {
    name: 'fetch: redirect Set-Cookie replays on same-host hops only',
    async run({ call, origin }) {
      const hopped = await call('browser::fetch', { url: `${origin}/hop-a`, include_html: true, retries: 1 })
      expectEqual(hopped.status, 200, 'hop status')
      expectEqual(hopped.url, `${origin}/hop-b`, 'hop final url')
      expect(hopped.html.includes('<p>hop=1</p>'), `redirect target did not receive the hop cookie: ${hopped.html}`)
      expectEqual(hopped.cookies, { hop: '1' }, 'envelope accumulates hop cookies')

      // 127.0.0.1 -> [::1] is the same handler on a different loopback
      // hostname: the Cookie header must not replay, while the envelope
      // accumulator (deliberately global across hops) still reports it.
      const scoped = await call('browser::fetch', { url: `${origin}/hop-x`, include_html: true, retries: 1 })
      expect(scoped.html.includes('<p>none</p>'), `cross-hostname hop leaked the cookie: ${scoped.html}`)
      expectEqual(scoped.cookies, { hopx: '1' }, 'accumulator still reports every hop cookie')
    },
  },
  {
    name: 'fetch: repeated response headers flatten with a comma join',
    async run({ call, origin }) {
      const r = await call('browser::fetch', { url: `${origin}/multi-cookie`, retries: 1 })
      expectEqual(r.headers['set-cookie'], 'a=1; Path=/, b=2; Path=/', 'set-cookie flattened')
      expectEqual(r.headers['x-test'], 'one, two', 'x-test flattened')
      expectEqual(r.cookies, { a: '1', b: '2' }, 'cookie map splits each set-cookie')
    },
  },
  {
    name: 'browser tiers: absurd durations clamp or refuse instead of panicking',
    async run({ call, origin }) {
      await expectError(
        () => call('browser::dynamic-fetch', { url: `${origin}/page`, timeout: -1 }),
        'handler error: Invalid argument type: Expected `float` >= 0.0 - at `$.timeout`',
        'negative timeout refused with the Python-parity message',
      )
      // Unclamped, Duration::from_secs_f64(1e300) panics inside the handler.
      const huge = await call('browser::dynamic-fetch', {
        url: `${origin}/page`,
        retry_delay: 1e300,
        retries: 1,
        timeout: 8000,
        include_html: true,
      })
      expectEqual(huge.status, 200, 'absurd retry_delay clamps instead of panicking')
    },
  },
  {
    name: 'safe mode: compat-only options are refused with the full policy message',
    async run({ call, origin }) {
      const url = `${origin}/page`
      const httpRefusals: Array<[Record<string, unknown>, string, string]> = [
        [
          { http3: true },
          'handler error: safe mode refuses `http3`: the safe reqwest engine has no certified HTTP/3 transport; use a certified compat build or remove the option',
          'http3',
        ],
        [
          { verify: false },
          'handler error: safe mode refuses `verify:false`: TLS certificate verification cannot be disabled; use a certified compat build or remove the option',
          'verify:false',
        ],
        [
          { proxies: { https: 'http://127.0.0.1:9/' } },
          'handler error: safe mode refuses `proxies`: per-scheme proxies bypass address pinning; use a certified compat build or remove the option',
          'proxies',
        ],
        [
          { proxy_auth: { username: 'u', password: 'p' } },
          'handler error: safe mode refuses `proxy_auth`: proxy authentication is unavailable when caller proxies are refused; use a certified compat build or remove the option',
          'proxy_auth',
        ],
        [
          { stealthy_headers: true },
          "handler error: safe mode refuses `stealthy_headers:true`: the safe reqwest engine cannot reproduce BrowserForge's generated header fingerprint; use a certified compat build or remove the option",
          'stealthy_headers:true',
        ],
        [
          { impersonate: 'firefox' },
          'handler error: safe mode refuses `impersonate`: the safe engine implements only its bounded Chrome header profile; use a certified compat build or remove the option',
          'impersonate',
        ],
      ]
      for (const [option, expected, label] of httpRefusals) {
        await expectError(() => call('browser::fetch', { url, ...option }), expected, `fetch ${label}`)
      }
      await expectError(
        () => call('browser::dynamic-fetch', { url, dns_over_https: true }),
        'handler error: dns_over_https require browser.scrapling.security_mode=compat',
        'dynamic-fetch dns_over_https',
      )
      await expectError(
        () => call('browser::session-open', { type: 'http', http3: true }),
        'handler error: session options require browser.scrapling.security_mode=compat; remove them or switch modes',
        'session-open compat-only option',
      )
    },
  },
  {
    name: 'safe mode: real_chrome is refused on every browser tier',
    async run({ call, origin }) {
      const url = `${origin}/page`
      for (const functionId of ['browser::dynamic-fetch', 'browser::stealthy-fetch', 'browser::screenshot-url']) {
        await expectError(
          () => call(functionId, { url, real_chrome: true }),
          'handler error: real_chrome require browser.scrapling.security_mode=compat',
          `${functionId} real_chrome`,
        )
      }
      await expectError(
        () => call('browser::session-open', { type: 'dynamic', real_chrome: true }),
        'handler error: session options require browser.scrapling.security_mode=compat; remove them or switch modes',
        'session-open real_chrome',
      )
      // Crawl validates per page inside the fetch closure, so the refusal is
      // an inline item error (unprefixed), not a top-level rejection.
      const crawled = await call('browser::crawl', {
        url,
        fetcher: 'dynamic',
        real_chrome: true,
        max_pages: 1,
        max_depth: 0,
        concurrency: 1,
      })
      expectEqual(crawled.stats.errors, 1, 'crawl real_chrome error count')
      expectEqual(
        crawled.items?.[0]?.error,
        'real_chrome require browser.scrapling.security_mode=compat',
        'crawl real_chrome inline item error',
      )
    },
  },
  {
    name: 'dynamic-fetch: dedicated worker resumes past waitForDebuggerOnStart',
    async run({ call, origin }) {
      // Child targets attach frozen (waitForDebuggerOnStart). Without the
      // child-target routing task the worker never runs and #out stays
      // "pending" forever.
      const r = await call('browser::dynamic-fetch', {
        url: `${origin}/worker-page`,
        include_html: true,
        wait: 1500,
        timeout: 10000,
        retries: 1,
      })
      expectEqual(r.status, 200, 'worker-page status')
      expect(r.html.includes('worker-ran'), `dedicated worker never resumed: ${r.html}`)
    },
  },
  {
    name: 'find: limit coerces floats and numeric strings, clamps negatives',
    async run({ call }) {
      const probe = async (limit: unknown) =>
        await call('browser::find', { html: LIMIT_HTML, tag: 'li', limit })
      const float = await probe(2.5)
      expectEqual(float.count, 5, 'count stays the pre-cap total')
      expectEqual(float.items.length, 2, 'float limit truncates toward zero')
      expectEqual((await probe('3')).items.length, 3, 'numeric string limit parses')
      expectEqual((await probe(' 2 ')).items.length, 2, 'whitespace-padded string limit parses')
      expectEqual((await probe(-1)).items.length, 0, 'negative limit clamps to zero')
      expectEqual((await probe(true)).items.length, 5, 'uncoercible limit falls back to the 100 cap')
      const first = await call('browser::find', { html: LIMIT_HTML, tag: 'li', limit: 5, first: true })
      expectEqual(first.items.length, 1, 'first:true short-circuits any limit')
    },
  },
  {
    name: 'describe: omitted kind is CSS, explicit null is XPath (Python parity)',
    async run({ call }) {
      // Omitted -> CSS branch: an XPath query is an invalid CSS selector.
      await expectError(
        () => call('browser::describe', { html: '<p>x</p>', query: '//p' }),
        "handler error: Invalid CSS selector '//p': Expected selector, got <DELIM '/' at 0>",
        'omitted kind takes the CSS branch',
      )
      // Explicit null -> XPath branch (Python: payload.get("kind", "css"),
      // where None != "css").
      const r = await call('browser::describe', { html: '<p>x</p>', query: '//p', kind: null })
      expectEqual(r.found, true, 'kind:null takes the XPath branch')
      expectEqual(r.element?.tag, 'p', 'xpath describe element')
    },
  },
  {
    name: 'parse: processing instructions are stripped from the tree',
    async run({ call }) {
      expectEqual(
        await call('browser::to-markdown', {
          html: "<html><body><?php echo 'LEAK'; ?><p>hi</p></body></html>",
          format: 'text',
        }),
        { format: 'text', content: 'hi' },
        'PI stripped from text rendering',
      )
      expectEqual(
        await call('browser::css', { html: '<div><?pi LEAK?>text</div>', query: 'div', first: true }),
        { result: 'text' },
        'PI stripped from css text extraction',
      )
    },
  },
  {
    name: 'to-markdown: whitespace, empty link attrs, list spacing, ol start',
    async run({ call }) {
      const md = async (html: string) =>
        (await call('browser::to-markdown', { html, format: 'markdown' })).content
      // ASCII runs collapse; NBSP survives (split_whitespace would eat it).
      expectEqual(await md('<h3>x\u00a0\u00a0y  \n z</h3>'), '### x\u00a0\u00a0y z', 'heading collapse keeps NBSP')
      const dt = await md('<dl><dt>Term  \n Name\u00a0X</dt><dd>Def</dd></dl>')
      expect(dt.includes('Term Name\u00a0X'), `dt collapse lost the NBSP or kept the run: ${JSON.stringify(dt)}`)
      // Empty href/title filter to None instead of producing link markup.
      expectEqual(await md('<p><a href="" title="">text</a></p>'), 'text', 'empty href drops link markup')
      expectEqual(await md('<p><a href="http://x/" title="">http://x/</a></p>'), '<http://x/>', 'empty title keeps the autolink form')
      // A bare text sibling after a list still forces the blank line.
      expectEqual(await md('<html><body><ul><li>a</li></ul>tail</body></html>'), '* a\n\ntail', 'list before bare text keeps the blank line')
      // ol start: ASCII digits only; anything else falls back to 1.
      expectEqual(await md('<ol start="١"><li>a</li></ol>'), '1. a', 'non-ASCII ol start falls back to 1')
      expectEqual(await md('<ol start="3"><li>a</li><li>b</li></ol>'), '3. a\n4. b', 'ASCII ol start numbers from it')
    },
  },
  {
    name: 'session: failed opens roll back their pending slot',
    async run({ call }) {
      // These fail INSIDE the constructor, after the pending counter was
      // reserved — exactly the path the drop guard protects. Without the
      // rollback, three failures would eat 3 of the 8 slots forever.
      for (let i = 0; i < 3; i++) {
        await expectError(
          () => call('browser::session-open', { type: 'dynamic', wait_selector_state: 'bogus' }),
          "handler error: Invalid argument type: Invalid enum value 'bogus' - at `$.wait_selector_state`",
          `failing open ${i + 1}`,
        )
      }
      const opened: string[] = []
      try {
        for (let i = 0; i < 8; i++) {
          const r = await call('browser::session-open', { type: 'http' })
          opened.push(r.session_id)
        }
        await expectError(
          () => call('browser::session-open', { type: 'http' }),
          'handler error: session limit reached (8); close one first',
          'capacity intact after failed opens',
        )
      } finally {
        for (const id of opened) await call('browser::session-close', { session_id: id })
      }
      // Closing freed the slots again.
      const again = await call('browser::session-open', { type: 'http' })
      expectEqual((await call('browser::session-close', { session_id: again.session_id })).closed, true, 'slot reusable after close')
    },
  },

  // ---- configuration-mutating cases: keep these LAST, restore in finally ---
  {
    name: 'inject_guidance: config flip binds and unbinds the pre-generate hook live',
    async run({ call }) {
      // The runner registers the harness::hook::pre-generate trigger type, so
      // the worker's boot-time binding activates from the engine's pending map
      // (asynchronously — hence a poll, not a one-shot check).
      await pollUntil(
        () => guidanceTriggerVisible(call),
        'the boot-time guidance binding to activate (inject_guidance defaults true)',
      )
      const original = await getBrowserConfig(call)
      try {
        const off = structuredClone(original)
        off.scrapling = { ...off.scrapling, inject_guidance: false }
        await setBrowserConfig(call, off)
        await pollUntil(
          async () => !(await guidanceTriggerVisible(call)),
          'guidance trigger to unbind after inject_guidance=false',
        )
        const on = structuredClone(original)
        on.scrapling = { ...on.scrapling, inject_guidance: true }
        await setBrowserConfig(call, on)
        await pollUntil(
          () => guidanceTriggerVisible(call),
          'guidance trigger to rebind after inject_guidance=true',
        )
      } finally {
        await setBrowserConfig(call, original)
      }
      // The hook itself: non-empty base appends the guidance, empty base
      // returns no mutation (preserves the harness prompt).
      const hooked = await call('browser::inject-guidance', { generate: { system_prompt: 'BASE' } })
      expect(
        typeof hooked.mutations?.system_prompt === 'string' &&
          hooked.mutations.system_prompt.startsWith('BASE\n\n## Showing a page vs scraping one (browser::*)'),
        `guidance hook did not append: ${JSON.stringify(hooked)}`,
      )
      expectEqual(
        await call('browser::inject-guidance', { generate: { system_prompt: '' } }),
        { mutations: {} },
        'empty base preserves the harness prompt',
      )
    },
  },
  {
    name: 'solve_cloudflare: the solve loop is bounded by the configured deadline',
    async run({ call, origin }) {
      const original = await getBrowserConfig(call)
      try {
        const lowered = structuredClone(original)
        lowered.max_timeout_ms = 3000
        await setBrowserConfig(call, lowered)
        await pollUntil(
          async () => (await getBrowserConfig(call)).max_timeout_ms === 3000,
          'the store to show the lowered timeout cap',
        )
        // configuration:updated reaches the worker asynchronously; if the
        // first attempt raced the reload it burns the SDK's 30s ceiling with
        // a different error, so allow one retry.
        await sleep(1500)
        let attempt = 0
        for (;;) {
          const start = Date.now()
          try {
            await expectError(
              () =>
                call('browser::stealthy-fetch', {
                  url: `${origin}/cf-managed`,
                  solve_cloudflare: true,
                  include_html: true,
                  retries: 1,
                }),
              'handler error: timed out solving the Cloudflare challenge',
              'unsolvable challenge times out',
            )
            expect(
              Date.now() - start < 20_000,
              'the solve deadline followed the lowered max_timeout_ms (3s), not the 60s floor',
            )
            break
          } catch (e) {
            if (++attempt >= 2) throw e
          }
        }
        // Control: same marker, no spin text — the solve loop falls through.
        const clean = await call('browser::stealthy-fetch', {
          url: `${origin}/cf-clean`,
          solve_cloudflare: true,
          include_html: true,
          retries: 1,
        })
        expectEqual(clean.status, 200, 'clean page passes the solve loop')
        expect(clean.html.includes('<p>done</p>'), `clean page html: ${clean.html}`)
      } finally {
        await setBrowserConfig(call, original)
      }
    },
  },
  {
    name: 'safe mode: the config-default proxy is not injected into requests',
    async run({ call, origin }) {
      const original = await getBrowserConfig(call)
      try {
        const withProxy = structuredClone(original)
        withProxy.scrapling = {
          ...withProxy.scrapling,
          defaults: { ...withProxy.scrapling.defaults, proxy: 'http://127.0.0.1:9/' },
        }
        await setBrowserConfig(call, withProxy)
        await pollUntil(
          async () => (await getBrowserConfig(call)).scrapling?.defaults?.proxy === 'http://127.0.0.1:9/',
          'the store to show the default proxy',
        )
        // Safe mode makes the omission itself invisible by design (that IS the
        // fix), so the barrier is store-side plus a settle delay: pre-fix,
        // both calls below failed against the dead proxy port.
        await sleep(1500)
        const http = await call('browser::fetch', { url: `${origin}/page`, retries: 1 })
        expectEqual(http.status, 200, 'safe fetch ignores the config-default proxy')
        const dyn = await call('browser::dynamic-fetch', { url: `${origin}/page`, retries: 1, timeout: 8000 })
        expectEqual(dyn.status, 200, 'safe dynamic-fetch ignores the config-default proxy')
        // The explicit caller refusal must survive the omission.
        await expectError(
          () => call('browser::fetch', { url: `${origin}/page`, proxy: 'http://127.0.0.1:9/' }),
          'handler error: safe mode refuses `proxy`: a caller proxy can resolve or route to addresses outside the egress policy; use a certified compat build or remove the option',
          'explicit caller proxy still refused',
        )
      } finally {
        await setBrowserConfig(call, original)
      }
    },
  },
  {
    name: 'crawl: allowed_domains are punycode-normalized before matching (IDN)',
    async run({ call, origin }) {
      // The /idn page links to the punycode host; the unicode allow-list only
      // admits it if both sides normalize to xn--mnchen-3ya.example. Admitted
      // means crawled (and failing DNS = an error item); filtered means the
      // frontier never grows.
      const strict = await call('browser::crawl', {
        url: `${origin}/idn`,
        fetcher: 'http',
        allowed_domains: ['münchen.example'],
        max_pages: 5,
        max_depth: 1,
        concurrency: 1,
        timeout: 5,
      })
      expectEqual(strict.stats.crawled, 2, 'unicode allow-list admits the punycode link')
      expectEqual(strict.stats.errors, 1, 'the admitted link fails to resolve')
      const errored = (strict.items ?? []).find((i: any) => i.url === 'http://xn--mnchen-3ya.example/')
      expect(Boolean(errored?.error), `expected an inline error item: ${JSON.stringify(strict.items)}`)

      const control = await call('browser::crawl', {
        url: `${origin}/idn`,
        fetcher: 'http',
        allowed_domains: ['nomatch.example'],
        max_pages: 5,
        max_depth: 1,
        concurrency: 1,
        timeout: 5,
      })
      expectEqual(control.stats.crawled, 1, 'non-matching allow-list filters the link')
      expectEqual(control.stats.errors, 0, 'filtered links produce no error')
    },
  },
]
