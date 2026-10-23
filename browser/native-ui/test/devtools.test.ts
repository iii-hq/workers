/** The console, network and history cards (design §8.4): seed then live, dedupe by seq, the 500 cap, filters, visibility. */
import type { TestHost } from '@iii-dev/native-ui/testing'
import { describe, expect, it } from 'vitest'
import { callsOf, FakeBrowser, start, subsOf, TABS } from './fake'

type Item = Record<string, unknown>

const entry = (seq: number, level = 'log', text = `message ${seq}`, source?: string) => ({
  seq,
  timestamp: Date.UTC(2026, 9, 22, 14, 3, 27) + seq * 1000,
  level,
  text,
  ...(source ? { source } : {}),
})

const items = (host: TestHost, list: string) => (host.find(list).items ?? []) as Item[]
const keys = (host: TestHost, list: string) => items(host, list).map((i) => i.key)

/** Delivers `payloads` of `trigger` in one host message (one VM turn). */
async function burst(host: TestHost, trigger: string, payloads: unknown[]) {
  const sub = [...host.subscriptions].find(([, s]) => s.trigger === trigger)?.[0]
  if (sub === undefined) throw new Error(`nothing bound to ${trigger}`)
  await host.deliver(payloads.map((p) => [9, sub, p]))
}

async function consoleOf(fake: FakeBrowser, ui?: NonNullable<Parameters<typeof start>[0]>['ui']) {
  const { host } = await start({ fake, ui })
  await host.mount('console', { session: 't1' })
  return host
}

describe('the console', () => {
  it('binds the tab’s events, then seeds the newest 200, one row per entry with its tone', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = [
      entry(1),
      entry(2, 'warning', 'deprecated API'),
      entry(3, 'exception', 'TypeError: x is undefined', 'https://example.com/app.js:12'),
      entry(4, 'debug'),
    ]
    const host = await consoleOf(fake)
    expect(host.tree()?.props).toMatchObject({ title: 'Console', subtitle: 'Example Domain' })
    expect(subsOf(host, 'browser::console-event')).toEqual([{ session_id: 't1' }])
    expect(callsOf(host, 'browser::console::read')).toEqual([{ session_id: 't1', limit: 200 }])
    expect(items(host, 'console-list')).toEqual([
      { key: '1', kind: 0, time: '14:03:28', level: 'log', text: 'message 1', source: '', hasSource: false },
      { key: '2', kind: 1, time: '14:03:29', level: 'warning', text: 'deprecated API', source: '', hasSource: false },
      {
        key: '3',
        kind: 2,
        time: '14:03:30',
        level: 'exception',
        text: 'TypeError: x is undefined',
        source: 'https://example.com/app.js:12',
        hasSource: true,
      },
      { key: '4', kind: 3, time: '14:03:31', level: 'debug', text: 'message 4', source: '', hasSource: false },
    ])
    // Four templates: the level's tone differs.
    const tones = host
      .findAll({ type: 'Text', role: 'smallStrong' }, { templates: true })
      .map((n) => n.props?.tone ?? 'default')
    expect(tones).toEqual(['default', 'warning', 'danger', 'faint'])
  })

  it('appends live entries, dropping seqs already seen and other tabs’ entries', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = [entry(1), entry(2)]
    const host = await consoleOf(fake)
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(3) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(3) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(2) })
    await host.emitTrigger('browser::console-event', { session_id: 't2', entry: entry(9) })
    expect(keys(host, 'console-list')).toEqual(['1', '2', '3'])
  })

  it('entries that arrive while the seed is out are kept after it, the seed’s own dropped', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = [entry(1), entry(2), entry(3)]
    fake.hold('browser::console::read')
    const { host } = await start({ fake })
    await host.mount('console', { session: 't1' })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(3) })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(4) })
    fake.release('browser::console::read')
    await host.settle()
    expect(keys(host, 'console-list')).toEqual(['1', '2', '3', '4'])
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(4) })
    expect(keys(host, 'console-list')).toEqual(['1', '2', '3', '4'])
  })

  it('keeps the newest 500', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = Array.from({ length: 250 }, (_, i) => entry(i + 1))
    const host = await consoleOf(fake)
    expect(keys(host, 'console-list')).toHaveLength(200)
    expect(keys(host, 'console-list')[0]).toBe('51')
    await burst(
      host,
      'browser::console-event',
      Array.from({ length: 400 }, (_, i) => ({ session_id: 't1', entry: entry(251 + i) })),
    )
    const all = keys(host, 'console-list')
    expect(all).toHaveLength(500)
    expect(all[0]).toBe('151')
    expect(all.at(-1)).toBe('650')
  })

  it('filters by level (errors include exceptions) and by text, seeding again', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = [entry(1), entry(2, 'error', 'boom'), entry(3, 'warning')]
    const host = await consoleOf(fake)
    await host.select('console-level', 1)
    expect(callsOf(host, 'browser::console::read').at(-1)).toEqual({ session_id: 't1', level: 'error', limit: 200 })
    expect(keys(host, 'console-list')).toEqual(['2'])
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(4, 'warning') })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(5, 'exception', 'Uncaught') })
    expect(keys(host, 'console-list')).toEqual(['2', '5'])

    await host.select('console-level', 0)
    await host.type('console-filter', 'BOOM')
    await host.tick(299)
    expect(callsOf(host, 'browser::console::read')).toHaveLength(3)
    await host.tick(1)
    // The worker gets the text as a literal, case-insensitive (its pattern is a Rust regex).
    expect(callsOf(host, 'browser::console::read').at(-1)).toEqual({
      session_id: 't1',
      pattern: '(?i)BOOM',
      limit: 200,
    })
    // The live filter is a case-insensitive substring, never a regex run in the VM.
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(6, 'log', 'big Boom') })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(7, 'log', 'quiet') })
    expect(keys(host, 'console-list').at(-1)).toBe('6')

    // Regex syntax is just text: escaped for the worker, matched literally live.
    await host.type('console-filter', 'a.b (x)|y')
    await host.tick(300)
    expect(host.find('console-filter').props?.error).toBeUndefined()
    expect(callsOf(host, 'browser::console::read').at(-1)?.pattern).toBe('(?i)a\\.b \\(x\\)\\|y')
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(8, 'log', 'aXb (x)|y') })
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(9, 'log', 'see A.B (X)|Y here') })
    expect(keys(host, 'console-list').at(-1)).toBe('9')
    expect(keys(host, 'console-list')).not.toContain('8')
  })

  it('unbinds while hidden and catches up from the last seq when visible', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = [entry(1), entry(2)]
    const host = await consoleOf(fake)
    await host.env({ visible: false })
    expect(subsOf(host, 'browser::console-event')).toEqual([])
    fake.consoleLog.push(entry(3), entry(4))
    await host.env({ visible: true })
    expect(subsOf(host, 'browser::console-event')).toEqual([{ session_id: 't1' }])
    expect(callsOf(host, 'browser::console::read').at(-1)).toEqual({ session_id: 't1', since_seq: 2, limit: 200 })
    expect(keys(host, 'console-list')).toEqual(['1', '2', '3', '4'])
  })

  it('Clear empties the list; a long press copies an entry; a failed read offers Retry', async () => {
    const fake = new FakeBrowser(TABS)
    fake.consoleLog = [entry(1), entry(2, 'log', 'copy me')]
    const host = await consoleOf(fake, { actionMenu: () => 0 })
    await host.itemLongPress('console-list', '2', 'entry')
    expect(host.clipboard).toEqual(['copy me'])
    await host.press('console-clear')
    expect(keys(host, 'console-list')).toEqual([])
    await host.emitTrigger('browser::console-event', { session_id: 't1', entry: entry(2) })
    expect(keys(host, 'console-list')).toEqual([])

    fake.overrides['browser::console::read'] = () => {
      throw new Error("tab 't1' is gone")
    }
    await host.select('console-level', 2)
    expect(host.find('console-error').props).toMatchObject({
      headline: "Couldn't read the console",
      detail: "tab 't1' is gone",
    })
    delete fake.overrides['browser::console::read']
    await host.press('console-retry')
    expect(host.findAll('console-error')).toEqual([])
  })
})

describe('the network', () => {
  const req = (seq: number, extra: Item = {}) => ({
    seq,
    timestamp: seq,
    method: 'get',
    url: `https://example.com/r${seq}`,
    status: 200,
    mime_type: 'text/html; charset=utf-8',
    failed: false,
    ...extra,
  })

  it('seeds, shows method, status with its tone, URL and type; failed only seeds again', async () => {
    const fake = new FakeBrowser(TABS)
    fake.network = [
      req(1),
      req(2, { failed: true, status: null, error: 'net::ERR_BLOCKED_BY_CLIENT' }),
      req(3, { status: null, mime_type: null }),
      req(4, { status: 404 }),
    ]
    const { host } = await start({ fake })
    await host.mount('network', { session: 't1' })
    expect(subsOf(host, 'browser::network-event')).toEqual([{ session_id: 't1' }])
    expect(items(host, 'network-list')).toEqual([
      { key: '1', kind: 0, method: 'GET', status: '200', url: 'https://example.com/r1', type: 'text/html' },
      {
        key: '2',
        kind: 1,
        method: 'GET',
        status: 'net::ERR_BLOCKED_BY_CLIENT',
        url: 'https://example.com/r2',
        type: 'text/html',
      },
      { key: '3', kind: 2, method: 'GET', status: '…', url: 'https://example.com/r3', type: '' },
      { key: '4', kind: 3, method: 'GET', status: '404', url: 'https://example.com/r4', type: 'text/html' },
    ])
    await host.toggle('failed-only')
    expect(callsOf(host, 'browser::network::read').at(-1)).toEqual({ session_id: 't1', limit: 200, failed_only: true })
    expect(keys(host, 'network-list')).toEqual(['2'])
    await host.emitTrigger('browser::network-event', { session_id: 't1', entry: req(5) })
    await host.emitTrigger('browser::network-event', { session_id: 't1', entry: req(6, { failed: true }) })
    expect(keys(host, 'network-list')).toEqual(['2', '6'])
  })
})

describe('the history', () => {
  it('lists the visits, searches after a pause, re-reads on navigation', async () => {
    const fake = new FakeBrowser(TABS)
    fake.visits = [
      { url: 'https://example.com/b', title: 'B', timestamp: Date.UTC(2026, 9, 22, 9, 30, 0) },
      { url: 'https://example.com/a', title: '', timestamp: Date.UTC(2026, 9, 22, 9, 0, 0) },
    ]
    const { host } = await start({ fake })
    await host.mount('history', { session: 't1' })
    expect(items(host, 'history-list')).toEqual([
      {
        key: `${fake.visits[0].timestamp}|https://example.com/b`,
        title: 'B',
        subtitle: 'example.com/b',
        time: '09:30:00',
        url: 'https://example.com/b',
      },
      {
        key: `${fake.visits[1].timestamp}|https://example.com/a`,
        title: 'example.com',
        subtitle: 'example.com/a',
        time: '09:00:00',
        url: 'https://example.com/a',
      },
    ])
    await host.type('history-search', '/a')
    await host.tick(300)
    expect(callsOf(host, 'browser::history::list').at(-1)).toEqual({ session_id: 't1', query: '/a' })
    expect(items(host, 'history-list')).toHaveLength(1)
    fake.visits.unshift({ url: 'https://example.com/a2', title: 'A2', timestamp: 3 })
    await host.emitTrigger('browser::navigated', { session_id: 't1', url: 'https://example.com/a2' })
    expect(items(host, 'history-list').map((i) => i.title)).toEqual(['A2', 'example.com'])
  })

  it('a refused read (allow_history_access) shows a panel with Retry', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::history::list'] = () => {
      throw new Error('history access is disabled (allow_history_access: false)')
    }
    const { host } = await start({ fake })
    await host.mount('history', { session: 't1' })
    expect(host.find('history-error').props?.detail).toBe('history access is disabled (allow_history_access: false)')
  })
})
