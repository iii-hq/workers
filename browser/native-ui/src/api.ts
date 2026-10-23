/**
 * Typed wrappers over the browser worker's functions and trigger types, as
 * the native app uses them (design §8; the wire source is
 * browser/src/functions/*.rs and browser/src/events.rs, the same ids and
 * payloads the console UI uses in ui/src/lib/browser.ts).
 */

import { iii, type JsonObject, type JsonValue, type Unsubscribe } from '@iii-dev/native-ui'

export const TRIGGER = {
  sessionStarted: 'browser::session-started',
  sessionStopped: 'browser::session-stopped',
  sessionUpdated: 'browser::session-updated',
  navigated: 'browser::navigated',
  consoleEvent: 'browser::console-event',
  networkEvent: 'browser::network-event',
  handoffRequested: 'browser::handoff-requested',
  handoffResolved: 'browser::handoff-resolved',
  installProgress: 'browser::chromium-install-progress',
} as const

/** The tab list re-reads on these: opened, closed, slept/woke/retitled, navigated. */
export const LIFECYCLE = [
  TRIGGER.sessionStarted,
  TRIGGER.sessionStopped,
  TRIGGER.sessionUpdated,
  TRIGGER.navigated,
] as const

/**
 * The RemoteScreen wire of the browser (design §6.7, exact): frames by
 * trigger, a seed read, a leased viewer that renews every 6 s, the resize
 * that makes the tab a phone the size of the view, and the input gestures
 * (a person's own: the worker draws no agent cursor for them).
 */
export const WIRE = {
  frames: { trigger: 'browser::frame-event', config: { session_id: '$session' } },
  seed: { fn: 'browser::frame', args: { session_id: '$session', since_frame: '$seq' } },
  start: {
    fn: 'browser::screencast::start',
    args: { session_id: '$session', viewer_id: '$viewer', lease_ms: 20000, scale: '$scale' },
    renewMs: 6000,
  },
  stop: { fn: 'browser::screencast::stop', args: { session_id: '$session', viewer_id: '$viewer' } },
  resize: {
    fn: 'browser::resize',
    args: {
      session_id: '$session',
      width: '$w',
      height: '$h',
      device_scale_factor: '$dpr',
      mobile: '$mobile',
      fit: true,
    },
    debounceMs: 180,
    minChangeDp: 4,
  },
  input: {
    fn: 'browser::act',
    tap: {
      session_id: '$session',
      action: 'click',
      x: '$x',
      y: '$y',
      button: 'left',
      click_count: '$count',
      ghost_cursor: false,
    },
    scroll: {
      session_id: '$session',
      action: 'scroll',
      x: '$x',
      y: '$y',
      delta_x: '$dx',
      delta_y: '$dy',
      ghost_cursor: false,
    },
    type: { session_id: '$session', action: 'type', text: '$text' },
    key: { session_id: '$session', action: 'press', key: '$key' },
  },
} as const satisfies JsonObject

// ── Payloads ────────────────────────────────────────────────────────────────

/** `SessionInfo` (browser/src/functions/sessions.rs). */
export interface SessionInfo {
  session_id: string
  url: string
  title?: string
  headless?: boolean
  read_only?: boolean
  incognito?: boolean
  /** False while the tab sleeps; opening it wakes it. */
  active?: boolean
  ttl_ms?: number
  created_ms?: number
  last_used_ms?: number
  console_entries?: number
}

export interface StartResult {
  session_id: string
  url: string
  headless?: boolean
  read_only?: boolean
  incognito?: boolean
  timed_out?: boolean
  error?: string
}

export interface NavigateResult {
  ok: boolean
  url: string
  title?: string
  timed_out?: boolean
  /** Chromium's error text when the page did not load (the tab shows its error page). */
  error?: string
}

export interface HistoryResult {
  ok: boolean
  url: string
  /** False when back/forward had no entry to move to. */
  moved: boolean
}

export interface ConsoleEntry {
  seq: number
  timestamp: number
  /** `log`, `info`, `warning`, `error`, `debug` or `exception`. */
  level: string
  text: string
  source?: string
}

export interface NetworkEntry {
  seq: number
  timestamp: number
  method: string
  url: string
  status?: number | null
  mime_type?: string | null
  failed: boolean
  error?: string | null
}

export interface LogRead<E> {
  entries: E[]
  last_seq: number
  dropped: number
}

export interface Visit {
  url: string
  title?: string
  timestamp: number
}

export interface FindResult {
  ok: boolean
  count: number
  /** 1-based; 0 when there is no match. */
  index: number
  query: string
}

export interface ScreenshotResult {
  content?: ({ type: 'image'; mime?: string; data?: string } | { type: 'text'; text?: string })[]
  details?: { url?: string; width?: number; height?: number }
}

export type InstallPhase = 'resolving' | 'downloading' | 'extracting' | 'verifying' | 'done' | 'failed'

/** `browser::chromium-install-progress` and `chromium::status().job`. */
export interface InstallProgress {
  job_id: string
  phase: InstallPhase
  version?: string
  bytes_done: number
  bytes_total?: number
  path?: string
  error?: string
  hint?: string
}

export interface ChromiumStatus {
  found: boolean
  path?: string
  version?: string
  /** `chromium` or `lightpanda`. */
  engine?: string
  install?: { supported: boolean; approx_download_mb?: number; reason?: string }
  job?: InstallProgress
}

export interface InstallResult {
  job_id?: string
  status: 'started' | 'running' | 'already-installed'
  path?: string
  version?: string
}

export interface SessionStopped {
  session_id: string
  /** `stopped`, `idle`, `expired` or `crashed`. */
  reason: string
}

export interface SessionUpdated {
  session_id: string
  active: boolean
  url: string
  title?: string
}

export interface Navigated {
  session_id: string
  url: string
}

export interface ConsoleEvent {
  session_id: string
  entry: ConsoleEntry
}

export interface NetworkEvent {
  session_id: string
  entry: NetworkEntry
}

export interface HandoffRequested {
  session_id: string
  handoff_id: string
  instructions: string
}

export interface HandoffResolved {
  session_id: string
  handoff_id: string
}

// ── Calls ───────────────────────────────────────────────────────────────────

/** navigate and history block until the page loads (the worker caps it at 30 s by default). */
const LOAD_TIMEOUT = { timeoutMs: 45_000 }

const call = <T>(fn: string, payload: JsonObject, opts?: { timeoutMs: number }) =>
  iii.call<T>(`browser::${fn}`, payload, opts)

export const browser = {
  list: () => call<{ sessions?: SessionInfo[] }>('sessions::list', {}).then((r) => r?.sessions ?? []),
  /** `preview: false`: this app shows the tab itself, consoles need not pop a preview of it. */
  start: (incognito: boolean) =>
    call<StartResult>('sessions::start', incognito ? { incognito: true, preview: false } : { preview: false }),
  stop: (session: string) => call<{ ok: boolean }>('sessions::stop', { session_id: session }),
  navigate: (session: string, url: string) =>
    call<NavigateResult>('navigate', { session_id: session, url }, LOAD_TIMEOUT),
  history: (session: string, action: 'back' | 'forward' | 'reload') =>
    call<HistoryResult>('history', { session_id: session, action }, LOAD_TIMEOUT),
  visits: (session: string, query: string) =>
    call<{ visits?: Visit[] }>('history::list', query ? { session_id: session, query } : { session_id: session }).then(
      (r) => r?.visits ?? [],
    ),
  consoleRead: (session: string, opts: { level?: string; pattern?: string; sinceSeq?: number; limit: number }) =>
    call<LogRead<ConsoleEntry>>(
      'console::read',
      compact({ session_id: session, ...readOpts(opts), level: opts.level }),
    ),
  networkRead: (session: string, opts: { failedOnly?: boolean; pattern?: string; sinceSeq?: number; limit: number }) =>
    call<LogRead<NetworkEntry>>(
      'network::read',
      compact({ session_id: session, ...readOpts(opts), failed_only: opts.failedOnly || undefined }),
    ),
  find: (session: string, query: string, action: 'search' | 'next' | 'previous' | 'close') =>
    call<FindResult>('find-in-page', { session_id: session, query, action }),
  zoom: (session: string, action: 'in' | 'out' | 'reset' | 'set' | 'read', level?: number) =>
    call<{ ok: boolean; level: number }>('zoom', compact({ session_id: session, action, level })).then(
      (r) => r?.level ?? 100,
    ),
  screenshot: (session: string) => call<ScreenshotResult>('screenshot', { session_id: session, format: 'png' }),
  clearData: (session: string) =>
    call<{ ok: boolean; cleared?: string[] }>('clear-data', {
      session_id: session,
      cookies: true,
      cache: true,
      storage: true,
    }).then((r) => r?.cleared ?? []),
  click: (session: string, x: number, y: number, button: 'left' | 'right') =>
    call<{ ok: boolean }>('act', { session_id: session, action: 'click', x, y, button }),
  handoffDone: (session: string, handoff: string) =>
    call<{ ok: boolean }>('handoff::confirm', { session_id: session, handoff_id: handoff }),
  chromiumStatus: () => call<ChromiumStatus>('chromium::status', {}),
  chromiumInstall: () => call<InstallResult>('chromium::install', {}),
}

function readOpts(opts: { pattern?: string; sinceSeq?: number; limit: number }): JsonObject {
  return compact({ pattern: opts.pattern || undefined, since_seq: opts.sinceSeq || undefined, limit: opts.limit })
}

/** `fields` without the undefined ones (the worker's inputs reject nothing, but the wire stays small). */
function compact(fields: Record<string, JsonValue | undefined>): JsonObject {
  const out: JsonObject = {}
  for (const [k, v] of Object.entries(fields)) if (v !== undefined) out[k] = v
  return out
}

/**
 * Binds trigger `type` of the browser (bind first, read after `ready`).
 * A failed binding is reported once and left unbound.
 */
export function listen<P>(
  type: string,
  config: JsonObject,
  handler: (payload: P) => void,
): Unsubscribe & {
  ready: Promise<boolean>
} {
  const off = iii.on<P>(type, config, (payload) => {
    if (payload && typeof payload === 'object') handler(payload)
  })
  const ready = off.ready.then(
    () => true,
    (err: unknown) => {
      iii.reportError(err)
      return false
    },
  )
  return Object.assign(() => off(), { ready })
}
