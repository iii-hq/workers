/**
 * A fake browser worker behind the test host: tabs, console, network,
 * history and Chromium state the calls read and change, and a way to hold
 * a call open (to see what the app shows while it waits).
 */

import { createTestHost, type Env, HostError, type TestHost, type TestHostOptions } from '@iii-dev/native-ui/testing'
import { inject } from 'vitest'

type Json = null | boolean | number | string | Json[] | { [key: string]: Json }
type Payload = Record<string, any>

export interface FakeTab {
  session_id: string
  url: string
  title?: string
  active?: boolean
  incognito?: boolean
  read_only?: boolean
}

export interface Held {
  resolve(value: unknown): void
  reject(err: Error): void
  payload: Payload
}

export class FakeBrowser {
  tabs: FakeTab[]
  next = 1
  chromium: { found: boolean; job?: Payload; supported?: boolean } = { found: true }
  consoleLog: Payload[] = []
  network: Payload[] = []
  visits: Payload[] = []
  zoom = 100
  /** Answers by function id that win over the fake's own. */
  overrides: Record<string, (payload: Payload) => unknown> = {}
  private held = new Map<string, Held[]>()
  private holding = new Set<string>()

  constructor(tabs: FakeTab[] = []) {
    this.tabs = tabs.map((t) => ({ ...t }))
  }

  /** The next calls of `fn` wait until `release(fn)` (or their Held) answers them. */
  hold(fn: string): void {
    this.holding.add(fn)
  }

  /** The calls of `fn` held so far, oldest first. */
  waiting(fn: string): Held[] {
    return this.held.get(fn) ?? []
  }

  /** Stops holding `fn`; resolves the oldest held call with `value` (or the fake's own answer). */
  release(fn: string, value?: unknown): void {
    this.holding.delete(fn)
    const h = this.held.get(fn)?.shift()
    if (h) h.resolve(value === undefined ? this.answer(fn, h.payload) : value)
  }

  calls(): TestHostOptions['calls'] {
    const ids = [
      'sessions::list',
      'sessions::start',
      'sessions::stop',
      'navigate',
      'history',
      'history::list',
      'console::read',
      'network::read',
      'find-in-page',
      'zoom',
      'screenshot',
      'clear-data',
      'act',
      'handoff::confirm',
      'chromium::status',
      'chromium::install',
    ]
    const out: Record<string, (payload: Json) => unknown> = {}
    for (const id of ids) {
      const fn = `browser::${id}`
      out[fn] = (payload) => {
        const p = (payload ?? {}) as Payload
        if (this.holding.has(fn)) {
          return new Promise((resolve, reject) => {
            const list = this.held.get(fn) ?? []
            list.push({ resolve, reject, payload: p })
            this.held.set(fn, list)
          })
        }
        return this.answer(fn, p)
      }
    }
    return out
  }

  answer(fn: string, p: Payload): unknown {
    const override = this.overrides[fn]
    if (override) return override(p)
    const tab = this.tabs.find((t) => t.session_id === p.session_id)
    switch (fn) {
      case 'browser::sessions::list':
        return {
          sessions: this.tabs.map((t) => ({
            headless: true,
            created_ms: 0,
            last_used_ms: 0,
            console_entries: 0,
            active: true,
            ...t,
          })),
        }
      case 'browser::sessions::start': {
        const t: FakeTab = { session_id: `n${this.next++}`, url: 'about:blank', active: true, incognito: !!p.incognito }
        this.tabs.push(t)
        return {
          session_id: t.session_id,
          url: t.url,
          headless: true,
          read_only: false,
          incognito: !!t.incognito,
          timed_out: false,
        }
      }
      case 'browser::sessions::stop':
        this.tabs = this.tabs.filter((t) => t.session_id !== p.session_id)
        return { ok: true, was_running: true }
      case 'browser::navigate':
        if (tab) tab.url = p.url
        return { ok: true, url: p.url, title: 'Loaded page', timed_out: false }
      case 'browser::history':
        return { ok: true, url: tab?.url ?? '', moved: true }
      case 'browser::history::list':
        return { visits: this.visits.filter((v) => !p.query || String(v.url).includes(p.query)) }
      case 'browser::console::read':
        return this.read(this.consoleLog, p, (e) => !p.level || e.level === p.level)
      case 'browser::network::read':
        return this.read(this.network, p, (e) => !p.failed_only || e.failed)
      case 'browser::find-in-page':
        return { ok: true, count: p.query ? 3 : 0, index: p.query ? (p.action === 'next' ? 2 : 1) : 0, query: p.query }
      case 'browser::zoom': {
        const ladder = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200]
        const i = ladder.indexOf(this.zoom)
        if (p.action === 'in') this.zoom = ladder[Math.min(i + 1, ladder.length - 1)]
        if (p.action === 'out') this.zoom = ladder[Math.max(i - 1, 0)]
        if (p.action === 'reset') this.zoom = 100
        if (p.action === 'set') this.zoom = p.level
        return { ok: true, level: this.zoom }
      }
      case 'browser::screenshot':
        return {
          content: [
            { type: 'image', mime: 'image/png', data: 'iVBORw0KGgo=' },
            { type: 'text', text: 'Screenshot of the page' },
          ],
          details: { session_id: p.session_id, url: tab?.url ?? '', width: 412, height: 800 },
        }
      case 'browser::clear-data':
        return { ok: true, cleared: ['cookies', 'cache', 'storage'] }
      case 'browser::act':
        return { ok: true, detail: '' }
      case 'browser::handoff::confirm':
        return { ok: true, handoff_id: p.handoff_id }
      case 'browser::chromium::status':
        return {
          found: this.chromium.found,
          engine: 'chromium',
          searched: [],
          install: {
            supported: this.chromium.supported ?? true,
            dir: '/cache',
            approx_download_mb: 150,
            ...(this.chromium.supported === false ? { reason: 'No build for this platform.' } : {}),
          },
          ...(this.chromium.job ? { job: this.chromium.job } : {}),
        }
      case 'browser::chromium::install':
        return { job_id: 'job1', status: 'started' }
      default:
        throw new HostError('function_not_found', `Function ${fn} not found`)
    }
  }

  private read(all: Payload[], p: Payload, keep: (e: Payload) => boolean) {
    const since = p.since_seq ?? 0
    const entries = all.filter((e) => e.seq > since && keep(e)).slice(-(p.limit ?? 100))
    const last = all.reduce((m, e) => Math.max(m, e.seq), 0)
    return { entries, last_seq: last, dropped: 0 }
  }
}

export interface Started {
  host: TestHost
  fake: FakeBrowser
}

/** A host running the app's bundle against `fake`, with the root page mounted. */
export async function start(
  options: {
    tabs?: FakeTab[]
    fake?: FakeBrowser
    env?: Partial<Env>
    ui?: TestHostOptions['ui']
    mount?: boolean
  } = {},
): Promise<Started> {
  const fake = options.fake ?? new FakeBrowser(options.tabs)
  const host = await createTestHost({
    bundle: inject('bundle'),
    ns: 'browser',
    calls: fake.calls(),
    env: options.env,
    ui: options.ui,
  })
  if (options.mount !== false) await host.mount('root')
  return { host, fake }
}

/** The payloads of every `call` of `fn`, in order. */
export function callsOf(host: TestHost, fn: string): Payload[] {
  return host.requests
    .filter((r) => r.method === 'call' && r.args.fn === fn)
    .map((r) => (r.args.payload ?? {}) as Payload)
}

/** The live subscriptions bound to `trigger`. */
export function subsOf(host: TestHost, trigger: string): Payload[] {
  return [...host.subscriptions.values()].filter((s) => s.trigger === trigger).map((s) => s.config as Payload)
}

/** The tab list's items as the host holds them. */
export function rows(host: TestHost): Payload[] {
  return (host.find('tabs').items ?? []) as Payload[]
}

export function exists(host: TestHost, testTag: string): boolean {
  return host.findAll(testTag).length > 0
}

/** Whether `testTag` is drawn: present and not `visible: false`. */
export function shown(host: TestHost, testTag: string): boolean {
  const all = host.findAll(testTag)
  return all.length > 0 && all.every((n) => n.props?.visible !== false)
}

export const TABS: FakeTab[] = [
  { session_id: 't1', url: 'https://example.com/', title: 'Example Domain' },
  { session_id: 't2', url: 'https://iii.dev/', title: 'iii — More application.', active: false },
  { session_id: 't3', url: 'https://github.com/login', title: 'Sign in to GitHub', incognito: true },
]
