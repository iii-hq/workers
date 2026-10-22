/**
 * The app's state (design §8.5): the tab list as rows ready to draw, the
 * selected tab, and per open tab what its page shows (address, title,
 * loading, zoom, find, editing, keyboard, handoffs, errors). Frames never
 * pass through here (D5). One VM per worker, so module state is the app's.
 */

import { createList, createSignal, iii } from '@iii-dev/native-ui'
import { displayUrl, isBlank, tabTitle } from './address'
import {
  browser,
  type HistoryResult,
  type Navigated,
  type SessionInfo,
  type SessionStopped,
  type SessionUpdated,
  TRIGGER,
} from './api'
import { readable } from './format'

type Signal<T> = ReturnType<typeof createSignal<T>>
export type Accessor<T> = Signal<T>[0]
export type Setter<T> = Signal<T>[1]

// ── The tab list ────────────────────────────────────────────────────────────

/** A row of the tab list. */
export interface TabRow {
  key: string
  /** The List's template: 0 an awake tab, 1 a group header, 2 a sleeping tab. */
  kind: 0 | 1 | 2
  title: string
  /** The address without its scheme (a tab row), or absent. */
  subtitle?: string
  icon?: string
  selected?: boolean
}

export const tabs = createList<TabRow>({ key: 'key', kind: 'kind' })

const [sessions, setSessions] = createSignal<readonly SessionInfo[]>([])
const [selected, setSelectedId] = createSignal<string | null>(null)
const [loaded, setLoaded] = createSignal(false)
const [listError, setListError] = createSignal<string | null>(null)
const [starting, setStarting] = createSignal(false)

export { listError, loaded, selected, sessions, starting }

export const sessionOf = (id: string): SessionInfo | undefined => sessions().find((s) => s.session_id === id)

/** Regular tabs, then incognito ones, each group in the worker's order. */
export function ordered(list: readonly SessionInfo[]): SessionInfo[] {
  return [...list.filter((s) => !s.incognito), ...list.filter((s) => s.incognito)]
}

/** The rows of the list: a header per group that has tabs, then its tabs. */
export function rowsOf(list: readonly SessionInfo[], sel: string | null): TabRow[] {
  const rows: TabRow[] = []
  const group = (label: string, key: string, items: SessionInfo[]) => {
    if (!items.length) return
    rows.push({ key, kind: 1, title: label })
    for (const s of items) {
      const asleep = s.active === false
      const row: TabRow = {
        key: s.session_id,
        kind: asleep ? 2 : 0,
        title: tabTitle(s.title, s.url),
        icon: asleep ? 'moon' : s.incognito ? 'eye-off' : 'globe',
        selected: s.session_id === sel,
      }
      const sub = displayUrl(s.url)
      if (sub) row.subtitle = sub
      rows.push(row)
    }
  }
  group(
    'Tabs',
    'header:tabs',
    list.filter((s) => !s.incognito),
  )
  group(
    'Incognito',
    'header:incognito',
    list.filter((s) => s.incognito),
  )
  return rows
}

/** A tab just opened, kept in the list until a read lists it (a read may have started before it). */
let held: SessionInfo | null = null
/** Tabs this app is closing: their `session-stopped` is no news. */
const closing = new Set<string>()
let reading = false
let readAgain = false
let refreshTimer: number | undefined

/** Reads `sessions::list` now (one read at a time; a request meanwhile reads once more after). */
export async function refresh(): Promise<void> {
  if (reading) {
    readAgain = true
    return
  }
  reading = true
  try {
    applyList(await browser.list())
    setListError(null)
  } catch (err) {
    setListError(readable(err))
  } finally {
    reading = false
    setLoaded(true)
    if (readAgain) {
      readAgain = false
      void refresh()
    }
  }
}

/** Every lifecycle event re-reads the list, 150 ms after the last one (design §8.4). */
export function refreshSoon(ms = 150): void {
  if (refreshTimer !== undefined) clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined
    void refresh()
  }, ms)
}

function applyList(fresh: SessionInfo[]): void {
  let list = fresh
  if (held) {
    const id = held.session_id
    if (list.some((s) => s.session_id === id)) held = null
    else list = [...list, held]
  }
  const ids = new Set(list.map((s) => s.session_id))
  for (const id of [...closing]) if (!ids.has(id)) closing.delete(id)
  // The open tab left the list without the event (missed while hidden): it was closed.
  const sel = selected()
  if (sel && !ids.has(sel)) openTabs.get(sel)?.setClosed((c) => c ?? 'closed')
  for (const s of list) {
    const tab = openTabs.get(s.session_id)
    if (!tab) continue
    if (s.url && tab.navigating === 0) tab.setUrl(s.url)
    tab.setTitle(s.title ?? '')
  }
  for (const id of [...openTabs.keys()]) if (!ids.has(id) && id !== sel) openTabs.delete(id)
  setSessions(list)
}

/** Opens tab `id` (the detail), or closes the detail with null. */
export function select(id: string | null): void {
  if (id) {
    const info = sessionOf(id)
    const tab = openTab(id)
    tab.waking = info?.active === false
    // The tab becomes a phone the size of the view when RemoteScreen
    // resizes it; a page loaded before that keeps the desktop user agent.
    if (tab.waking || !seen.has(id)) watchMode(id)
    seen.add(id)
    if (info) {
      tab.setUrl(info.url)
      tab.setTitle(info.title ?? '')
    }
  }
  setSelectedId(id)
}

/** Ids in the order the list shows them. */
export function tabOrder(): string[] {
  return ordered(sessions()).map((s) => s.session_id)
}

/** `sessions::start`, then the new tab opens with its address field ready (design §8.4). */
export async function newTab(incognito = false): Promise<void> {
  if (starting()) return
  setStarting(true)
  try {
    const started = await browser.start(incognito)
    const info: SessionInfo = {
      session_id: started.session_id,
      url: started.url || 'about:blank',
      incognito: started.incognito ?? incognito,
      read_only: started.read_only,
      active: true,
    }
    held = info
    setSessions((list) => (list.some((s) => s.session_id === info.session_id) ? list : [...list, info]))
    // A blank tab has nothing loaded with the old user agent.
    seen.add(info.session_id)
    select(info.session_id)
    openTab(info.session_id).setEditing(true)
    refreshSoon(0)
  } catch (err) {
    iii.ui.toast(readable(err), 'error')
  } finally {
    setStarting(false)
  }
}

/** `sessions::stop`; the open tab moves to its right neighbour, else the left one, else the list. */
export async function closeTab(id: string): Promise<void> {
  if (selected() === id) {
    const order = tabOrder()
    const i = order.indexOf(id)
    select(order[i + 1] ?? order[i - 1] ?? null)
  }
  closing.add(id)
  if (held?.session_id === id) held = null
  setSessions((list) => list.filter((s) => s.session_id !== id))
  try {
    await browser.stop(id)
  } catch (err) {
    closing.delete(id)
    iii.ui.toast(`Couldn't close the tab: ${readable(err)}`, 'error')
  } finally {
    refreshSoon(0)
  }
}

/** A lifecycle event (bound without a filter): the open tab hears its own, the list re-reads. */
export function lifecycle(type: string, payload: SessionStopped | SessionUpdated | Navigated): void {
  const id = payload.session_id
  const tab = typeof id === 'string' ? openTabs.get(id) : undefined
  if (tab) {
    if (type === TRIGGER.sessionStopped) {
      if (id === selected() && !closing.has(id)) tab.setClosed((payload as SessionStopped).reason || 'closed')
    } else if (type === TRIGGER.sessionUpdated) {
      updated(tab, payload as SessionUpdated)
    } else if (type === TRIGGER.navigated) {
      navigated(tab, (payload as Navigated).url)
    }
  }
  refreshSoon()
}

// ── The open tab ────────────────────────────────────────────────────────────

export type HistoryAction = 'back' | 'forward' | 'reload'

export interface Handoff {
  id: string
  instructions: string
}

/** What the page of an open tab shows besides the live view. */
export interface OpenTab {
  readonly id: string
  url: Accessor<string>
  setUrl: Setter<string>
  title: Accessor<string>
  setTitle: Setter<string>
  /** The indeterminate bar: a navigate in flight, or a navigation the page made (design §8.4). */
  loading: Accessor<boolean>
  setLoading: Setter<boolean>
  /** CSS zoom in percent; the worker's zoom resets on navigation, so it is applied again. */
  zoom: Accessor<number>
  setZoom: Setter<number>
  /** The find bar: null when closed. */
  find: Accessor<{ query: string; count: number; index: number } | null>
  setFind: Setter<{ query: string; count: number; index: number } | null>
  editing: Accessor<boolean>
  setEditing: Setter<boolean>
  keyboard: Accessor<boolean>
  setKeyboard: Setter<boolean>
  handoffs: Accessor<readonly Handoff[]>
  setHandoffs: Setter<readonly Handoff[]>
  /** The last navigate that did not load: the panel with Retry. */
  error: Accessor<{ url: string; detail: string } | null>
  setError: Setter<{ url: string; detail: string } | null>
  /** Why the tab stopped while open (`crashed`, `expired`, …), or null. */
  closed: Accessor<string | null>
  setClosed: Setter<string | null>
  /** The history button waiting for its answer. */
  busy: Accessor<HistoryAction | null>
  setBusy: Setter<HistoryAction | null>
  /** Opened asleep: the live view wakes it. */
  waking: boolean
  /** navigate calls of this app in flight. */
  navigating: number
  /** The 3 s fallback of a navigation the page made. */
  settleTimer?: number
}

const openTabs = new Map<string, OpenTab>()
/** Tabs this VM has opened before. */
const seen = new Set<string>()

export function openTab(id: string): OpenTab {
  let tab = openTabs.get(id)
  if (tab) return tab
  const [url, setUrl] = createSignal('')
  const [title, setTitle] = createSignal('')
  const [loading, setLoading] = createSignal(false)
  const [zoom, setZoom] = createSignal(100)
  const [find, setFind] = createSignal<{ query: string; count: number; index: number } | null>(null)
  const [editing, setEditing] = createSignal(false)
  const [keyboard, setKeyboard] = createSignal(false)
  const [handoffs, setHandoffs] = createSignal<readonly Handoff[]>([])
  const [error, setError] = createSignal<{ url: string; detail: string } | null>(null)
  const [closed, setClosed] = createSignal<string | null>(null)
  const [busy, setBusy] = createSignal<HistoryAction | null>(null)
  tab = {
    id,
    url,
    setUrl,
    title,
    setTitle,
    loading,
    setLoading,
    zoom,
    setZoom,
    find,
    setFind,
    editing,
    setEditing,
    keyboard,
    setKeyboard,
    handoffs,
    setHandoffs,
    error,
    setError,
    closed,
    setClosed,
    busy,
    setBusy,
    waking: false,
    navigating: 0,
  }
  openTabs.set(id, tab)
  return tab
}

/** The page the app loads: the bar shows while the call is out. */
export async function navigate(tab: OpenTab, url: string): Promise<void> {
  tab.navigating++
  tab.setLoading(true)
  tab.setError(null)
  tab.setUrl(url)
  try {
    const result = await browser.navigate(tab.id, url)
    // A page that did not load keeps the address it was asked for (the tab shows Chromium's error page).
    if (result && result.ok === false) tab.setError({ url, detail: result.error || 'The page did not load.' })
    else {
      if (result?.url) tab.setUrl(result.url)
      if (result?.title) tab.setTitle(result.title)
    }
  } catch (err) {
    // A refusal (scheme or origin policy) throws: a toast, the address stays as it was.
    iii.ui.toast(readable(err), 'error')
    const info = sessionOf(tab.id)
    if (info) tab.setUrl(info.url)
  } finally {
    tab.navigating--
    if (tab.navigating === 0) tab.setLoading(false)
  }
}

const NO_MOVE: Record<HistoryAction, string> = {
  back: 'No earlier page',
  forward: 'No later page',
  reload: '',
}

/** Back, forward, reload: the button waits as a spinner; nothing to move to says so. */
export async function history(tab: OpenTab, action: HistoryAction): Promise<HistoryResult | null> {
  if (tab.busy()) return null
  tab.setBusy(action)
  tab.setError(null)
  try {
    const result = await browser.history(tab.id, action)
    if (result?.url) tab.setUrl(result.url)
    if (result && !result.moved && NO_MOVE[action]) iii.ui.toast(NO_MOVE[action])
    return result
  } catch (err) {
    iii.ui.toast(readable(err), 'error')
    return null
  } finally {
    tab.setBusy(null)
  }
}

/** A navigation the page made (a click), not one of ours: the bar until the new title, or 3 s. */
function navigated(tab: OpenTab, url: string): void {
  if (typeof url === 'string' && url) tab.setUrl(url)
  if (tab.navigating === 0) {
    tab.setLoading(true)
    if (tab.settleTimer !== undefined) clearTimeout(tab.settleTimer)
    tab.settleTimer = setTimeout(() => settle(tab), 3000)
  }
  const level = tab.zoom()
  if (level !== 100) browser.zoom(tab.id, 'set', level).catch(() => {})
}

function updated(tab: OpenTab, p: SessionUpdated): void {
  if (typeof p.url === 'string' && p.url && tab.navigating === 0) tab.setUrl(p.url)
  if (typeof p.title === 'string') tab.setTitle(p.title)
  if (p.active === false) {
    // Asleep: it wakes as a desktop, and the live view makes it a phone again.
    tab.waking = true
    watchMode(tab.id)
  }
  if (tab.settleTimer !== undefined) settle(tab)
}

function settle(tab: OpenTab): void {
  if (tab.settleTimer !== undefined) clearTimeout(tab.settleTimer)
  tab.settleTimer = undefined
  if (tab.navigating === 0) tab.setLoading(false)
}

// ── Mobile mode (design §9.3) ───────────────────────────────────────────────

/**
 * Tabs whose next resize by the live view may switch them to the phone
 * mode, with the width of their first frame since: the new user agent only
 * reaches the page on its next load, so the app reloads such a tab once.
 */
const modeWatch = new Map<string, number | null>()

function watchMode(id: string): void {
  modeWatch.set(id, null)
}

/**
 * RemoteScreen's `frame` event. A host that passes the resize's
 * `mode_changed` as a third argument decides it; otherwise a watched tab
 * reloads when its frames get narrower than the first one (the desktop a
 * new or waking tab starts as, then the view's width).
 */
export function framed(tab: OpenTab, width: number, modeChanged?: unknown): void {
  if (typeof modeChanged === 'boolean') {
    modeWatch.delete(tab.id)
    if (modeChanged) reloadForMode(tab)
    return
  }
  if (!modeWatch.has(tab.id) || typeof width !== 'number') return
  const first = modeWatch.get(tab.id)
  if (first == null) modeWatch.set(tab.id, width)
  else if (width < first) {
    modeWatch.delete(tab.id)
    reloadForMode(tab)
  }
}

function reloadForMode(tab: OpenTab): void {
  if (isBlank(tab.url()) || tab.navigating > 0) return
  tab.setLoading(true)
  browser
    .history(tab.id, 'reload')
    .catch(() => {})
    .finally(() => {
      if (tab.navigating === 0) tab.setLoading(false)
    })
}
