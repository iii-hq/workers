/**
 * Adversarial pass over the Browser app's mobile UI script (browser/native-ui).
 * Each test asserts the correct behaviour. A test that the current code fails
 * is skipped with its finding number, so this file passes as committed; the
 * findings are listed in the commit message and the report.
 */
import type { TestHost } from '@iii-dev/native-ui/testing'
import { describe, expect, it } from 'vitest'
import { callsOf, exists, FakeBrowser, type FakeTab, rows, start, subsOf, TABS } from './fake'

type Options = Parameters<typeof start>[0]

/** The VM heap budget the catalog sets (packages/native-ui/catalog.json: heapBytesDefault). */
const HEAP_BUDGET = 64 * 1024 * 1024

const loading = (host: TestHost) => host.find('loading').props?.visible !== false

/** Answers the action menus in order. */
function answers(...list: number[]) {
  return { actionMenu: () => list.shift() ?? -1 }
}

async function opened(id = 't1', options: Options = {}) {
  const started = await start({ tabs: TABS, ...options })
  await started.host.itemPress('tabs', id)
  return started
}

/** Types `text` into the address bar and submits it. */
async function go(host: TestHost, text: string) {
  await host.press('address')
  await host.type('address-field', text)
  await host.submit('address-field')
}

const navigates = (host: TestHost) => callsOf(host, 'browser::navigate').map((p) => String(p.url))

const keysOf = (host: TestHost, list: string) =>
  (host.find(list).items ?? []).map((r) => (r as { key: string }).key)

/** Delivers `count` events of `trigger` (to every subscription bound to it), in chunks. */
async function flood(host: TestHost, trigger: string, make: (i: number) => unknown, count: number, chunk = 500) {
  const subs = [...host.subscriptions].filter(([, s]) => s.trigger === trigger).map(([id]) => id)
  for (let i = 0; i < count; i += chunk) {
    const entries: unknown[][] = []
    for (let j = i; j < Math.min(count, i + chunk); j++) for (const sub of subs) entries.push([9, sub, make(j) as never])
    await host.deliver(entries)
  }
}

const consoleEntry = (seq: number, text = `line ${seq}`, level = 'log') => ({
  seq,
  timestamp: seq,
  level,
  text,
  source: 'app.js:1',
})

const networkEntry = (seq: number, extra: Record<string, unknown> = {}) => ({
  seq,
  timestamp: seq,
  method: 'GET',
  url: `https://a.example/${seq}`,
  status: 200,
  mime_type: 'text/html',
  failed: false,
  ...extra,
})

describe('adversarial: the address bar', () => {
  // finding 1: the bar sends any `scheme://` to browser::navigate; the worker's allowlist is the only gate.
  it.skip('javascript: and data: addresses never reach browser::navigate', async () => {
    const { host } = await opened('t1')
    await go(host, 'javascript://alert(1)')
    await go(host, 'data://text/html,<script>alert(1)</script>')
    expect(navigates(host).filter((u) => !/^https?:\/\//.test(u))).toEqual([])
  })

  // finding 2: chrome:, intent: and about: pass the bar unchanged.
  it.skip('browser-internal and app-launching addresses never reach browser::navigate', async () => {
    const { host } = await opened('t1')
    await go(host, 'chrome://settings')
    await go(host, 'intent://example.com/#Intent;scheme=https;end')
    await go(host, 'about:config')
    expect(navigates(host).filter((u) => !/^https?:\/\//.test(u))).toEqual([])
  })

  // finding 3: file: is sent, and the worker's default allowlist (http, https, file) accepts it.
  // The existing 'a refused address' test relies on the worker refusing file:; the owner must decide where the gate sits.
  it.skip('a file: address is refused by the bar, not sent to the worker', async () => {
    const { host } = await opened('t1')
    await go(host, 'file:///etc/hosts')
    expect(navigates(host)).toEqual([])
  })

  // held: a credential in the URL names its real host; a bracketed IPv6 local host gets http.
  it('a URL with credentials names its real host; a bracketed IPv6 local host gets http', async () => {
    const { host } = await opened('t1')
    await go(host, 'iii.dev@evil.example')
    expect(host.find('tab').props?.subtitle).toBe('evil.example')
    await go(host, '[::1]:8080')
    expect(navigates(host).at(-1)).toBe('http://[::1]:8080')
  })

  // finding 17: NUL, BEL and ESC pass trim() and reach navigate.
  it.skip('control characters in an address never reach browser::navigate', async () => {
    const { host } = await opened('t1')
    await go(host, 'example.com\u0000')
    await go(host, 'exa\u0007mple.com')
    await go(host, 'example.com/\u001b[31mred')
    expect(navigates(host).filter((u) => /[\u0000-\u001f\u007f]/.test(u))).toEqual([])
  })

  // finding 18: an address with no host is still navigated.
  it.skip('an address with no host is not navigated', async () => {
    const { host } = await opened('t1')
    await go(host, 'https://')
    await go(host, 'https://?q=1')
    expect(navigates(host)).toEqual([])
  })

  // held: a 100 KB address neither crashes the VM nor leaves the bar stuck.
  it('a 100 KB address keeps the app alive and the bar usable', async () => {
    const { host } = await opened('t1')
    await go(host, `${'x'.repeat(100_000)}.example`)
    expect(host.failed).toBeNull()
    expect(loading(host)).toBe(false)
    expect(exists(host, 'address')).toBe(true)
  })

  // held: a bare javascript:alert(1) becomes an https URL, never a script scheme.
  it('a bare javascript:alert(1) is not sent as a script scheme', async () => {
    const { host } = await opened('t1')
    await go(host, 'javascript:alert(1)')
    expect(navigates(host).every((u) => u.startsWith('https://'))).toBe(true)
  })
})

describe('adversarial: data from the worker', () => {
  // finding 4: a history visit's URL goes to navigate without the address rules.
  it.skip('a history pick of a javascript: URL is not navigated', async () => {
    const fake = new FakeBrowser(TABS)
    fake.visits = [{ url: 'javascript:alert(1)', title: 'x', timestamp: 1 }]
    const { host } = await opened('t1', { fake, ui: answers(7) })
    await host.press('tab-menu')
    await host.itemPress('history-list', '1|javascript:alert(1)')
    expect(navigates(host)).toEqual([])
  })

  // finding 5: sessions::list is sent without timeoutMs, so the documented 30 s default never applies, and
  // `reading` (state.ts refresh) never clears: the tab list stops reading for good.
  it.skip('a hung sessions::list is given up at the call timeout, and the list reads again', async () => {
    const { host, fake } = await start({ tabs: TABS })
    fake.hold('browser::sessions::list')
    await host.emit('tabs', 'refresh')
    await host.tick(30_000)
    expect(host.find('list-error').props?.detail).toBe("The engine didn't answer in time")
    fake.release('browser::sessions::list')
    await host.press('list-retry')
    expect(exists(host, 'list-error')).toBe(false)
    expect(rows(host)).toHaveLength(5)
  })

  // finding 6: sessions::start is sent without timeoutMs; `starting` (state.ts newTab) never clears, so New tab stays disabled.
  it.skip('a hung sessions::start is given up at the call timeout, and New tab comes back', async () => {
    const { host, fake } = await start({ tabs: TABS })
    fake.hold('browser::sessions::start')
    await host.press('new-tab')
    expect(host.find('new-tab').props?.enabled).toBe(false)
    await host.tick(30_000)
    expect(host.toasts.at(-1)?.kind).toBe('error')
    expect(host.find('new-tab').props?.enabled).toBe(true)
  })

  // finding 7: a non-string title makes tabTitle throw in rowsOf; the whole tab list goes blank, with no error.
  it.skip('a session whose title is not a string does not break the tab list', async () => {
    const odd = { session_id: 'odd', url: 'https://odd.example/', title: 42 } as unknown as FakeTab
    const { host } = await start({ fake: new FakeBrowser([odd, ...TABS]) })
    expect(host.failed).toBeNull()
    expect(rows(host).find((r) => r.key === 'odd')).toMatchObject({ title: 'odd.example' })
  })

  // finding 8: one null entry throws in applyList; every tab is replaced by "Couldn't read the tabs".
  it.skip('a null entry in the session list does not hide the other tabs', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::sessions::list'] = () => ({ sessions: [null, ...TABS] })
    const { host } = await start({ fake })
    expect(exists(host, 'list-error')).toBe(false)
    expect(rows(host).map((r) => r.key)).toEqual(['header:tabs', 't1', 't2', 'header:incognito', 't3'])
  })

  // finding 9: a handoff is shown in the top slot whatever the tab's state; it stays after the tab stopped.
  it.skip('a handoff banner is cleared when the open tab stops', async () => {
    const { host } = await opened('t1')
    await host.emitTrigger('browser::handoff-requested', { session_id: 't1', handoff_id: 'h1', instructions: 'Sign in' })
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'crashed', timestamp: 1 })
    expect(exists(host, 'handoff')).toBe(false)
  })

  // finding 10: the load-error panel is not gated on the tab being closed; it stays after the tab stopped.
  it.skip('a load error is not shown on a tab that stopped', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::navigate'] = (p) => ({ ok: false, url: p.url, error: 'net::ERR_ABORTED' })
    const { host } = await opened('t1', { fake })
    await go(host, 'nope.invalid')
    expect(exists(host, 'load-error')).toBe(true)
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'crashed', timestamp: 2 })
    expect(exists(host, 'load-error')).toBe(false)
  })

  // finding 11: a tab held for the first read and stopped before any read lists it stays as a row.
  it.skip('a new tab stopped before the list lists it is not kept as a row', async () => {
    const fake = new FakeBrowser(TABS)
    const { host } = await start({ fake })
    fake.hold('browser::sessions::list')
    await host.press('new-tab')
    fake.overrides['browser::sessions::list'] = () => ({ sessions: TABS })
    fake.release('browser::sessions::list')
    await host.settle()
    await host.emitTrigger('browser::session-stopped', { session_id: 'n1', reason: 'expired', timestamp: 1 })
    await host.tick(200)
    expect(rows(host).map((r) => r.key)).not.toContain('n1')
  })

  // finding 12: the live filter runs the user's JS regex in the VM; a catastrophic pattern runs for seconds, unbounded.
  it.skip('a catastrophic filter pattern does not stall the app on a live entry', async () => {
    const { host } = await opened('t1', { ui: answers(5) })
    await host.press('tab-menu')
    await host.settle()
    await host.type('console-filter', '(a+)+$')
    await host.tick(300)
    const t0 = Date.now()
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: consoleEntry(1, `${'a'.repeat(26)}!`) })
    expect(Date.now() - t0).toBeLessThan(1000)
    expect(host.failed).toBeNull()
  })

  // finding 13: a start answer without a session_id opens a keyless, selected tab; no error is shown.
  it.skip('a start answer without a session_id is a toast, and no tab opens', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::sessions::start'] = () => ({ url: 'https://new.example/' })
    const { host } = await start({ fake })
    await host.press('new-tab')
    expect(host.failed).toBeNull()
    expect(host.toasts.at(-1)?.kind).toBe('error')
    expect(rows(host).every((r) => typeof r.key === 'string')).toBe(true)
  })

  // finding 14: two entries with one session_id are two rows with one List key (the test host reports a protocol violation).
  it.skip('two entries with the same session_id show one row', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::sessions::list'] = () => ({ sessions: [TABS[0], TABS[0], ...TABS.slice(1)] })
    const { host } = await start({ fake })
    const keys = rows(host).map((r) => r.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  // finding 15: a read answer with a duplicated seq puts two rows under one List key.
  it.skip('a console read answer with a duplicated seq does not break the list', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::console::read'] = () => ({
      entries: [consoleEntry(1), consoleEntry(1), consoleEntry(2)],
      last_seq: 2,
      dropped: 0,
    })
    const { host } = await opened('t1', { fake, ui: answers(5) })
    await host.press('tab-menu')
    await host.settle()
    expect(host.failed).toBeNull()
    const keys = keysOf(host, 'console-list')
    expect(new Set(keys).size).toBe(keys.length)
  })

  // finding 16: a session without a session_id is listed under an undefined key (not selectable, no error).
  it.skip('a session without a session_id is skipped, not listed under an undefined key', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::sessions::list'] = () => ({ sessions: [{ url: 'https://nokey.example/' }, ...TABS] })
    const { host } = await start({ fake })
    expect(host.failed).toBeNull()
    expect(rows(host).every((r) => typeof r.key === 'string')).toBe(true)
  })

  // finding 19: a long press with non-numeric coordinates reaches browser::act.
  it.skip('a long press with non-numeric coordinates sends no click', async () => {
    const { host } = await opened('t1', { ui: answers(0) })
    await host.emit('screen', 'longPress', ['abc', null])
    expect(callsOf(host, 'browser::act').filter((p) => typeof p.x !== 'number' || typeof p.y !== 'number')).toEqual([])
  })

  // finding 20: a menu left open while the tab stopped still zooms the dead session when its choice is made.
  it.skip('a menu choice after the tab stopped sends no zoom to it', async () => {
    let answer: (value: number) => void = () => {}
    const { host } = await opened('t1', {
      ui: { actionMenu: () => new Promise<number>((resolve) => (answer = resolve)) },
    })
    void host.press('tab-menu')
    await host.settle()
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'crashed', timestamp: 1 })
    answer(1)
    await host.settle()
    expect(callsOf(host, 'browser::zoom').filter((p) => p.action !== 'read')).toEqual([])
  })

  // finding 21: a non-string mime_type throws in networkRow; the live row is dropped and a TypeError reported.
  it.skip('a network entry with a non-string mime_type is still listed', async () => {
    const { host } = await opened('t1', { ui: answers(6) })
    await host.press('tab-menu')
    await host.settle()
    await host.emitTrigger('browser::network-event', { session_id: 't1', entry: networkEntry(1, { mime_type: 42 }) })
    expect(keysOf(host, 'network-list')).toEqual(['1'])
  })

  // finding 23: a stop reason naming an Object property (constructor, toString) is looked up in the CLOSED map.
  it.skip('a stop reason that names an Object property shows the generic closed title', async () => {
    const { host } = await opened('t1')
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'constructor', timestamp: 1 })
    expect(host.find('tab-closed').props?.title).toBe('This tab was closed')
  })

  // finding 24: control characters and bidi overrides reach the title as they came.
  it.skip('control characters and bidi overrides are removed from a tab title', async () => {
    const { host } = await start({
      tabs: [{ session_id: 'bidi', url: 'https://example.org/', title: 'Ex‮ample\u0007 Domain\u001b[0m' }],
    })
    const title = String(rows(host).find((r) => r.key === 'bidi')?.title ?? '')
    expect(title).not.toMatch(/[\u0000-\u001f\u007f‪-‮⁦-⁩]/)
  })

  // held: a console event for another tab is ignored; duplicates show once, late ones are dropped.
  it('console events of another tab are ignored; duplicates show once, late ones are dropped', async () => {
    const { host } = await opened('t1', { ui: answers(5) })
    await host.press('tab-menu')
    await host.settle()
    await host.emitTrigger('browser::console-event', { session_id: 't2', entry: consoleEntry(1) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: consoleEntry(1) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: consoleEntry(1) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: consoleEntry(3) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: consoleEntry(2) })
    expect(keysOf(host, 'console-list')).toEqual(['1', '3'])
  })

  // held: a flood of navigated events for one tab makes one list read, not one per event.
  it('a flood of navigated events makes one list read, not one per event', async () => {
    const { host } = await opened('t1')
    const before = callsOf(host, 'browser::sessions::list').length
    for (let i = 0; i < 200; i++) {
      await host.emitTrigger('browser::navigated', { session_id: 't1', url: `https://example.com/${i}`, timestamp: i })
    }
    await host.tick(200)
    expect(callsOf(host, 'browser::sessions::list').length - before).toBeLessThanOrEqual(1)
    expect(host.failed).toBeNull()
  })
})

describe('adversarial: memory', () => {
  // finding 22: while the first console read is out, every live entry is kept (`early`), unbounded.
  it.skip('a page logging 40,000 lines while the first console read is out stays within the heap budget', async () => {
    const { host, fake } = await opened('t1', { ui: answers(5) })
    fake.hold('browser::console::read')
    await host.press('tab-menu')
    const text = 'x'.repeat(2000)
    await flood(host, 'browser::console-event', (i) => ({ session_id: 't1', entry: consoleEntry(i + 1, text) }), 40_000)
    expect(host.failed).toBeNull()
    expect(host.memory().memory_used_size).toBeLessThan(HEAP_BUDGET)
    fake.release('browser::console::read')
    await host.settle()
    expect(host.failed).toBeNull()
    expect(keysOf(host, 'console-list').length).toBeLessThanOrEqual(500)
  })

  // held: the console keeps at most 500 rows, the newest, under a flood.
  it('a flood of 3,000 console lines keeps 500 rows, the newest', async () => {
    const { host } = await opened('t1', { ui: answers(5) })
    await host.press('tab-menu')
    await host.settle()
    await flood(host, 'browser::console-event', (i) => ({ session_id: 't1', entry: consoleEntry(i + 1) }), 3000)
    const keys = keysOf(host, 'console-list')
    expect(keys).toHaveLength(500)
    expect(keys.at(-1)).toBe('3000')
  })
})

describe('adversarial: races and leaks', () => {
  // held: a find answer that lands after the bar closed leaves no bar behind.
  it('a find search answered after the find bar closed leaves no bar', async () => {
    const { host, fake } = await opened('t1', { ui: answers(0) })
    await host.press('tab-menu')
    fake.hold('browser::find-in-page')
    await host.type('find-field', 'domain')
    await host.tick(300)
    await host.press('find-close')
    fake.release('browser::find-in-page')
    fake.release('browser::find-in-page')
    await host.settle()
    expect(exists(host, 'find-bar')).toBe(false)
    expect(exists(host, 'address-bar')).toBe(true)
  })

  // held: switching tabs during a find leaves the find bar on the first tab only.
  it('switching tabs during a find leaves the find bar on the first tab only', async () => {
    const { host, fake } = await opened('t1', { ui: answers(0) })
    await host.press('tab-menu')
    fake.hold('browser::find-in-page')
    await host.type('find-field', 'domain')
    await host.tick(300)
    await host.itemPress('tabs', 't2')
    fake.release('browser::find-in-page')
    await host.settle()
    expect(exists(host, 'find-bar')).toBe(false)
    expect(exists(host, 'address-bar')).toBe(true)
    expect(host.find('tab').props?.title).toBe('iii — More application.')
  })

  // held: a navigate answered after its tab closed changes no other tab.
  it('a navigate answered after its tab closed changes no other tab', async () => {
    const { host, fake } = await opened('t1')
    fake.hold('browser::navigate')
    await go(host, 'iii.dev/docs')
    await host.itemPress('tabs', 't1', 'tab-close')
    fake.release('browser::navigate')
    await host.settle()
    expect(host.failed).toBeNull()
    expect(host.find('screen').props?.session).toBe('t2')
    expect(host.find('tab').props?.subtitle).toBe('iii.dev')
    expect(loading(host)).toBe(false)
    expect(exists(host, 'load-error')).toBe(false)
  })

  // held: after the root unmounts, a pending answer is harmless and no subscription remains.
  it('after the root unmounts, a pending navigate answers quietly and no subscription remains', async () => {
    const { host, fake } = await opened('t1')
    fake.hold('browser::navigate')
    await go(host, 'iii.dev')
    await host.unmount(host.deck[0].inst)
    fake.release('browser::navigate')
    await host.settle()
    expect(host.exceptions).toEqual([])
    expect(host.failed).toBeNull()
    expect(host.subscriptions.size).toBe(0)
  })

  // held: toggling visibility never doubles a subscription; leaving a tab drops its own.
  it('visibility toggles keep one subscription per trigger, and leaving a tab drops its own', async () => {
    const { host } = await opened('t1')
    for (let i = 0; i < 3; i++) {
      await host.env({ visible: false })
      await host.env({ visible: true })
    }
    expect(subsOf(host, 'browser::navigated')).toHaveLength(1)
    expect(subsOf(host, 'browser::handoff-requested')).toHaveLength(1)
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'crashed', timestamp: 1 })
    await host.press('back-to-tabs')
    expect(subsOf(host, 'browser::handoff-requested')).toEqual([])
    expect(subsOf(host, 'browser::console-event')).toEqual([])
  })

  // held: leaving the console card drops its event binding.
  it('leaving the console card drops its event binding', async () => {
    const { host } = await opened('t1', { ui: answers(5) })
    await host.press('tab-menu')
    expect(subsOf(host, 'browser::console-event')).toHaveLength(1)
    await host.back()
    expect(subsOf(host, 'browser::console-event')).toEqual([])
  })

  // held: every call and every subscription the app makes is in its own browser:: namespace.
  it('every call and trigger the app makes is in browser::', async () => {
    const fake = new FakeBrowser(TABS)
    const { host } = await start({ fake, ui: { ...answers(5, 6, 7), confirm: () => true } })
    await host.itemPress('tabs', 't1')
    await go(host, 'javascript://x')
    await go(host, 'chrome://settings')
    await host.press('back')
    await host.press('reload')
    await host.press('keyboard')
    await host.press('tab-menu')
    await host.back()
    await host.press('tab-menu')
    await host.back()
    await host.press('tab-menu')
    await host.back()
    await host.press('new-tab')
    await host.itemPress('tabs', 'n1', 'tab-close')
    const calls = host.requests.filter((r) => r.method === 'call').map((r) => String(r.args.fn))
    expect(calls.length).toBeGreaterThan(5)
    expect(calls.filter((fn) => !fn.startsWith('browser::'))).toEqual([])
    expect([...host.subscriptions.values()].filter((s) => !s.trigger.startsWith('browser::'))).toEqual([])
  })
})
