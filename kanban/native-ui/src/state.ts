/**
 * The app's state (design §10): the board as lanes, the configuration that
 * came with it, the agents, projects and releases, the filters, the last
 * visit and this app's own writes, the lane on screen and the ticket open in
 * the root's detail; and the actions on them: reading the board and its
 * pages, the live `kanban:change` binding, moving a card. One VM per worker,
 * so module state is the app's.
 */

import { batch, createSignal, IiiError, iii } from '@iii-dev/native-ui'
import {
  type Agent,
  type BoardRead,
  type ChangeEvent,
  CONFIG_EVENT,
  type Column,
  FIRST_PAGE,
  kanban,
  listen,
  type Project,
  type Release,
  type Summary,
  type Ticket,
} from './api'
import {
  allRows,
  appendPage,
  applyRead,
  applyWrite,
  findRow,
  isComplete,
  type Lane,
  newMemo,
  partial,
  placeMove,
  type Write,
  type WriteKind,
} from './board'
import { coerceFilters, EMPTY_FILTERS, type Filters, isEmpty, type MatchContext, validVisit } from './filters'

// ── Words ───────────────────────────────────────────────────────────────────

export const OFFLINE = 'The engine is not connected'

/** The person-facing reason of a failed call: the engine's words, never JSON. */
export function readable(err: unknown): string {
  if (err instanceof IiiError) {
    switch (err.code) {
      case 'timeout':
        return "The engine didn't answer in time"
      case 'not_connected':
      case 'connection_lost':
        return OFFLINE
      case 'function_not_found':
        return "The kanban worker isn't running on the engine"
    }
  }
  const message = err instanceof Error ? err.message : typeof err === 'string' ? err : ''
  return clean(message) || 'Something went wrong'
}

function clean(message: string): string {
  const text = message.trim()
  if (text.startsWith('{')) {
    try {
      const v = JSON.parse(text) as Record<string, unknown>
      const inner = v.message ?? v.error
      if (typeof inner === 'string') return clean(inner)
    } catch {
      // Not JSON after all.
    }
  }
  return text.replace(/^(Error|IiiError):\s*/, '')
}

export const toast = (text: string, kind?: 'info' | 'success' | 'error') => iii.ui.toast(text, kind)
export const online = () => iii.env.online

// ── The board ───────────────────────────────────────────────────────────────

export interface BoardMeta {
  columns: Column[]
  priorities: string[]
  defaultStatus: string
  done: string
  canceled: string | null
  prefix: string
}

const [lanes, setLanes] = createSignal<Lane[]>([])
const [meta, setMeta] = createSignal<BoardMeta | null>(null)
const [loaded, setLoaded] = createSignal(false)
const [loadError, setLoadError] = createSignal<string | null>(null)
const [agents, setAgents] = createSignal<Agent[]>([])
const [projects, setProjects] = createSignal<Project[]>([])
const [releases, setReleases] = createSignal<Release[]>([])
const [paging, setPaging] = createSignal<ReadonlySet<string>>(new Set())
/** Read the whole board from now on: a filter is (or was) on. */
const [wantAll, setWantAll] = createSignal(false)
const [readingAll, setReadingAll] = createSignal(false)
const [now, setNow] = createSignal(Date.now())

export { agents, lanes, loadError, loaded, meta, now, paging, projects, readingAll, releases, setNow }

const memo = newMemo()

export const complete = () => isComplete(lanes())
export const columns = () => meta()?.columns ?? []

/** The writes that arrive while a read is out, replayed over it (the console UI's journal). */
let pending = 0
let journal: Write[] = []
const begin = () => {
  pending += 1
  return journal.length
}
const end = () => {
  pending -= 1
  if (pending === 0) journal = []
}
/** Lanes whose page is out and that a write changed meanwhile. */
const touched = new Set<string>()

/** A card as the worker has it (an event, a write's answer). */
export function write(summary: Summary, kind: WriteKind = {}): void {
  if (pending) journal.push({ summary, kind })
  for (const id of paging()) touched.add(id)
  setLanes((current) => applyWrite(current, summary, memo, kind))
}

/** The card fields of a ticket (a write answers with the whole ticket). */
export function summaryOf(t: Ticket): Summary {
  return {
    id: t.id,
    key: t.key,
    title: t.title,
    status: t.status,
    priority: t.priority,
    assignee: t.assignee,
    labels: t.labels,
    pills: t.pills,
    order: t.order,
    created_at: t.created_at,
    updated_at: t.updated_at,
    deleted_at: t.deleted_at,
    comment_count: t.comment_count,
    parent_id: t.parent_id,
    project_id: t.project_id,
    release_id: t.release_id,
  }
}

function applyMeta(read: BoardRead): void {
  setMeta({
    columns: read.columns.map((c) => ({ id: c.id, label: c.label })),
    priorities: read.priorities.length ? read.priorities : ['low', 'medium', 'high', 'urgent'],
    defaultStatus: read.default_status,
    done: read.done_status,
    canceled: read.canceled_status,
    prefix: read.id_prefix,
  })
  setProjects(read.projects)
  setReleases(read.releases)
}

let loadSeq = 0

/**
 * Reads the board: every row with a filter on (the filters need them all),
 * else as many rows per column as the app shows now (at least the first
 * page). Only the newest read is applied; `quiet` reads show no error over a
 * board already on screen. Resolves false when the read failed.
 */
export async function load(quiet = false): Promise<boolean> {
  const token = ++loadSeq
  const from = begin()
  const all = wantAll()
  const shown = Math.max(
    0,
    ...lanes()
      .filter(partial)
      .map((lane) => lane.rows.length),
  )
  const limit = Math.ceil(Math.max(FIRST_PAGE, shown) / FIRST_PAGE) * FIRST_PAGE
  if (all) setReadingAll(true)
  try {
    const read = await kanban.board(all ? undefined : limit)
    if (token !== loadSeq) return false
    batch(() => {
      applyMeta(read)
      setLanes(applyRead(read, journal.slice(from), memo))
      setLoadError(null)
      setLoaded(true)
    })
    void readVisit()
    return true
  } catch (err) {
    if (token !== loadSeq) return false
    if (!quiet || !meta()) setLoadError(readable(err))
    setLoaded(true)
    return false
  } finally {
    end()
    if (token === loadSeq) setReadingAll(false)
  }
}

/** The whole board, once: the filters see every card from now on. */
export function readAll(): void {
  if (wantAll() && (complete() || readingAll())) return
  setWantAll(true)
  if (!complete()) void load(true)
}

/** The next page of a partial lane (`endReached`, Load more). */
export async function more(id: string): Promise<void> {
  const lane = lanes().find((l) => l.id === id)
  if (!lane || !partial(lane) || paging().has(id) || !online()) return
  setPaging((s) => new Set(s).add(id))
  touched.delete(id)
  const had = lane.rows.length
  try {
    const page = await kanban.page(id, had)
    const next = appendPage(lanes(), id, page.rows, page.total, memo, touched.has(id))
    setLanes(next)
    const after = next.find((l) => l.id === id)
    // Nothing new while still partial: the rows are out of step with the column.
    if (after && partial(after) && after.rows.length === had) void load(true)
  } catch (err) {
    toast(`Couldn't read more of ${lane.label}: ${readable(err)}`, 'error')
  } finally {
    touched.delete(id)
    setPaging((s) => {
      const next = new Set(s)
      next.delete(id)
      return next
    })
  }
}

let planTimer: number | undefined

/** A card names a project or release the app has not heard of: read the board again, after the burst. */
function planSoon(): void {
  if (planTimer !== undefined) clearTimeout(planTimer)
  planTimer = setTimeout(() => {
    planTimer = undefined
    void load(true)
  }, 400)
}

export async function loadAgents(): Promise<void> {
  try {
    setAgents(await kanban.agents())
  } catch {
    // The board shows ids instead of names; the next visible read tries again.
  }
}

// ── Live ────────────────────────────────────────────────────────────────────

const listeners = new Set<(event: ChangeEvent) => void>()

/** The ticket screens hear every event the board applies. */
export function onChange(listener: (event: ChangeEvent) => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function onEvent(event: ChangeEvent): void {
  if (event.event === CONFIG_EVENT) void load(true)
  else if (event.ticket) {
    const s = event.ticket
    write(s, {
      moved: event.event === 'ticket.moved',
      entered: event.event === 'ticket.created' || event.event === 'ticket.restored',
      from: event.from,
    })
    if (
      (s.project_id && !projects().some((p) => p.id === s.project_id)) ||
      (s.release_id && !releases().some((r) => r.id === s.release_id))
    )
      planSoon()
  }
  for (const listener of [...listeners]) {
    try {
      listener(event)
    } catch (err) {
      iii.reportError(err)
    }
  }
}

/** Binds `kanban:change`, then reads the board (bind first, read after `ready`). The cleanup unbinds. */
export function goLive(): () => void {
  const off = listen(onEvent)
  let bound = true
  void off.ready.then(() => {
    if (bound) void load(!!meta())
  })
  return () => {
    bound = false
    off()
  }
}

// ── Moving ──────────────────────────────────────────────────────────────────

/** The writes of one card go one after the other: a second drop must not overtake the first. */
const tails = new Map<string, Promise<unknown>>()

/**
 * Moves card `id` to `status` at `index` (absent = the end of the column):
 * on screen at once, then the worker; a failure says why, reads the board
 * again, and puts the lanes back as they were when that read fails too.
 * Resolves with the ticket the worker returned, or null.
 */
export function moveTicket(id: string, status: string, index?: number): Promise<Ticket | null> {
  const row = findRow(lanes(), id)
  if (!online()) {
    toast(OFFLINE, 'error')
    return Promise.resolve(null)
  }
  const before = lanes()
  setLanes(placeMove(before, id, status, index))
  const save = async (): Promise<Ticket | null> => {
    const from = begin()
    try {
      const ticket = await kanban.move(id, status, index)
      noteOwn(ticket)
      write(summaryOf(ticket), { moved: true })
      // Another card changed while this one was out (its place was drawn against a guess), or nothing changed.
      const crowded = journal.slice(from).some((w) => w.summary.id !== id)
      if (crowded || (row && Date.parse(ticket.updated_at) <= Date.parse(row.updated_at))) void load(true)
      return ticket
    } catch (err) {
      toast(`Couldn't move ${row?.key ?? 'the ticket'}: ${readable(err)}`, 'error')
      if (!(await load(true))) setLanes(before)
      return null
    } finally {
      end()
    }
  }
  const job = (tails.get(id) ?? Promise.resolve()).then(save)
  tails.set(id, job)
  void job.then(() => {
    if (tails.get(id) === job) tails.delete(id)
  })
  return job
}

// ── Own writes and the last visit ───────────────────────────────────────────

const own = new Map<string, string>()
const [ownVersion, setOwnVersion] = createSignal(0)

/** A write this app made (the ticket as the worker returned it): not "changed since your last visit". */
export function noteOwn(ticket: { id: string; updated_at: string }): void {
  const known = own.get(ticket.id)
  if (known !== undefined && Date.parse(known) >= Date.parse(ticket.updated_at)) return
  own.set(ticket.id, ticket.updated_at)
  setOwnVersion((v) => v + 1)
}

/** A ticket write of this app: noted as its own, and on the board at once. */
export function wrote(ticket: Ticket): void {
  noteOwn(ticket)
  write(summaryOf(ticket))
}

const [lastVisit, setLastVisit] = createSignal<string | null>(null)
let visitRead = false

/** The previous visit, read once per session (when the prefix is known). */
async function readVisit(): Promise<void> {
  const prefix = meta()?.prefix
  if (visitRead || prefix === undefined) return
  visitRead = true
  try {
    setLastVisit(validVisit(await iii.storage.get(`visit:${prefix}`)))
  } catch {
    // No visit: no dot.
  }
}

/** The app leaves the screen: this is the last visit for the next session. */
export function stampVisit(): void {
  const prefix = meta()?.prefix
  if (prefix === undefined) return
  iii.storage.set(`visit:${prefix}`, new Date(Date.now()).toISOString()).catch(() => {})
}

/** What the filters and the "changed" dot compare against. */
export const matchContext = (): MatchContext => {
  ownVersion()
  return { lastVisit: lastVisit(), now: now(), own }
}

// ── Filters, the lane on screen, the open ticket ────────────────────────────

const [filters, setFiltersSignal] = createSignal<Filters>(EMPTY_FILTERS)
const [lane, setLaneSignal] = createSignal<string | null>(null)
const [selected, setSelected] = createSignal<string | null>(null)

export { filters, lane, selected, setSelected }

export const filtering = () => !isEmpty(filters())

let saveTimer: number | undefined

export function setFilters(next: Filters): void {
  setFiltersSignal(next)
  if (!isEmpty(next)) readAll()
  if (saveTimer !== undefined) clearTimeout(saveTimer)
  saveTimer = setTimeout(() => {
    saveTimer = undefined
    iii.storage.set('filters', next as never).catch(() => {})
  }, 500)
}

export function setLane(id: string): void {
  if (lane() === id) return
  setLaneSignal(id)
  iii.storage.set('lane', id).catch(() => {})
}

/** Opens a card in the root's detail, its lane on screen in the same batch (D6). */
export function openCard(row: { key: string; status: string }): void {
  batch(() => {
    setLaneSignal(row.status)
    setSelected(row.key)
  })
}

/** Filters and the lane as they were left on this device. */
export async function restore(): Promise<void> {
  try {
    const [stored, storedLane] = await Promise.all([iii.storage.get('filters'), iii.storage.get('lane')])
    const f = coerceFilters(stored)
    if (!isEmpty(f)) {
      setFiltersSignal(f)
      setWantAll(true)
    }
    if (typeof storedLane === 'string' && !lane()) setLaneSignal(storedLane)
  } catch {
    // Nothing kept: the defaults.
  }
}

// ── Lookups ─────────────────────────────────────────────────────────────────

export const agentOf = (id: string | null | undefined) => (id ? agents().find((a) => a.id === id) : undefined)
export const nameOf = (id: string) => agentOf(id)?.name ?? (id === 'user' ? 'You' : id)
export const boardRows = () => allRows(lanes())
