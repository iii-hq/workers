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
  upsert,
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
/** The filters page is on screen: its counts need every card. */
const [filtersOpen, setFiltersOpen] = createSignal(false)
const [readingAll, setReadingAll] = createSignal(false)
/** Read the whole board: a filter is on, or the filters page is open (after Clear all, reads page again). */
const wantAll = () => filtering() || filtersOpen()
/** How far a lane was paged (the first page, then each Load more): a quiet read keeps that many rows. */
let pagedTo = FIRST_PAGE
const [now, setNow] = createSignal(Date.now())

export { agents, lanes, loadError, loaded, meta, now, paging, projects, readingAll, releases, setFiltersOpen, setNow }

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

export type LoadResult = 'applied' | 'superseded' | 'failed'

/**
 * Reads the board: every row with a filter on (the filters need them all),
 * else as many rows per column as the person paged to (at least the first
 * page). Only the newest read is applied (an older one resolves
 * `superseded`); the moves still out are drawn again over it; `quiet` reads
 * show no error over a board already on screen.
 */
export async function load(quiet = false): Promise<LoadResult> {
  const token = ++loadSeq
  const from = begin()
  const all = wantAll()
  const limit = Math.ceil(Math.max(FIRST_PAGE, pagedTo) / FIRST_PAGE) * FIRST_PAGE
  if (all) setReadingAll(true)
  try {
    const read = await kanban.board(all ? undefined : limit)
    if (token !== loadSeq) return 'superseded'
    batch(() => {
      applyMeta(read)
      let next = applyRead(read, journal.slice(from), memo)
      // A move the worker has not answered yet: this read was answered before it, so it is drawn again.
      for (const [id, move] of optimistic) {
        if (memo.versions.get(id) === move.version) next = placeMove(next, id, move.status, move.index, memo.placed)
      }
      setLanes(next)
      setLoadError(null)
      setLoaded(true)
    })
    void readVisit()
    return 'applied'
  } catch (err) {
    if (token !== loadSeq) return 'superseded'
    if (!quiet || !meta()) setLoadError(readable(err))
    setLoaded(true)
    return 'failed'
  } finally {
    end()
    if (token === loadSeq) setReadingAll(false)
  }
}

/** The whole board, when a filter or the filters page needs it. A failure shows (the filters would be short). */
export function readAll(): void {
  if (complete() || readingAll()) return
  void load()
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
    const changed = touched.has(id)
    const next = appendPage(lanes(), id, page.rows, page.total, memo, changed)
    setLanes(next)
    const after = next.find((l) => l.id === id)
    if (after) pagedTo = Math.max(pagedTo, after.rows.length)
    // The lane changed while the page was out (its offset may have skipped or repeated a row), or nothing new
    // came while it is still partial: the rows are out of step with the column. One read sets them right.
    if (changed || (after && partial(after) && after.rows.length === had)) void load(true)
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
  // An event the app cannot read still says the board changed: read it again, after the burst.
  const off = listen(onEvent, planSoon)
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
/** Moves on screen that the worker has not answered yet, with the card's version when they were made. */
const optimistic = new Map<string, { status: string; index?: number; version: number | undefined }>()

/**
 * Moves card `id` to `status` at `index` (absent = the end of the column):
 * on screen at once, then the worker; a failure says why and reads the board
 * again, and when that read fails too, puts this card back where it was
 * (unless a newer write of it came meanwhile). Resolves with the ticket the
 * worker returned, or null.
 */
export function moveTicket(id: string, status: string, index?: number, from?: string): Promise<Ticket | null> {
  const row = findRow(lanes(), id)
  // The column it leaves, for the counts when the board does not hold the card (a partial lane).
  const left = row?.status ?? from
  if (!online()) {
    toast(OFFLINE, 'error')
    return Promise.resolve(null)
  }
  const at = row
    ? (lanes()
        .find((l) => l.id === row.status)
        ?.rows.findIndex((r) => r.id === id) ?? -1)
    : -1
  const version = memo.versions.get(id)
  const pending = { status, index, version }
  optimistic.set(id, pending)
  setLanes((current) => placeMove(current, id, status, index, memo.placed))
  const save = async (): Promise<Ticket | null> => {
    const from = begin()
    try {
      const ticket = await kanban.move(id, status, index)
      if (optimistic.get(id) === pending) optimistic.delete(id)
      noteOwn(ticket)
      write(summaryOf(ticket), { moved: true, from: left })
      // Another card changed while this one was out (its place was drawn against a guess), or nothing changed.
      const crowded = journal.slice(from).some((w) => w.summary.id !== id)
      if (crowded || (row && Date.parse(ticket.updated_at) <= Date.parse(row.updated_at))) void load(true)
      return ticket
    } catch (err) {
      if (optimistic.get(id) === pending) optimistic.delete(id)
      toast(`Couldn't move ${row?.key ?? 'the ticket'}: ${readable(err)}`, 'error')
      if ((await load(true)) === 'failed' && row && at >= 0 && memo.versions.get(id) === version) {
        setLanes((current) => upsert(current, { ...row, order: at + 1 }, { moved: true }, memo.placed))
      }
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

/**
 * A ticket write of this app: noted as its own, and on the board at once.
 * `kind` says what the write did to its column (a create or a restore
 * enters it; a move names the column it left), so a partial lane counts it.
 */
export function wrote(ticket: Ticket, kind: WriteKind = {}): void {
  noteOwn(ticket)
  write(summaryOf(ticket), kind)
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

/** Storage keeps at most 64 KiB a value (limits.maxStorageValueBytes): what is saved stays well under. */
const SAVED_MAX = 32_768

/** UTF-8 bytes of `text` (no TextEncoder in the VM). */
function utf8(text: string): number {
  let bytes = 0
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i)
    if (code < 0x80) bytes += 1
    else if (code < 0x800) bytes += 2
    else if (code >= 0xd800 && code < 0xdc00) {
      // A surrogate pair is one 4-byte character.
      bytes += 4
      i++
    } else bytes += 3
  }
  return bytes
}

/** The filters as saved: a set too big to keep (a pasted wall of text, thousands of labels) is cut down. */
export function saveable(f: Filters): { value: Filters; trimmed: boolean } {
  if (utf8(JSON.stringify(f)) <= SAVED_MAX) return { value: f, trimmed: false }
  const cut = (values: string[]) => values.slice(0, 50).map((v) => v.slice(0, 100))
  const value: Filters = {
    ...f,
    q: f.q.slice(0, 200),
    labels: cut(f.labels),
    assignees: cut(f.assignees),
    priorities: cut(f.priorities),
    projects: cut(f.projects),
    releases: cut(f.releases),
  }
  return { value, trimmed: true }
}

export function setFilters(next: Filters): void {
  setFiltersSignal(next)
  if (!isEmpty(next)) readAll()
  if (saveTimer !== undefined) clearTimeout(saveTimer)
  saveTimer = setTimeout(() => {
    saveTimer = undefined
    const { value, trimmed } = saveable(next)
    if (trimmed) toast('These filters are too long to keep on this device; a shorter version is kept')
    iii.storage
      .set('filters', value as never)
      .catch((err: unknown) => toast(`Couldn't keep the filters: ${readable(err)}`, 'error'))
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
    if (!isEmpty(f)) setFiltersSignal(f)
    if (typeof storedLane === 'string' && !lane()) setLaneSignal(storedLane)
  } catch {
    // Nothing kept: the defaults.
  }
}

// ── Lookups ─────────────────────────────────────────────────────────────────

export const agentOf = (id: string | null | undefined) => (id ? agents().find((a) => a.id === id) : undefined)
export const nameOf = (id: string) => agentOf(id)?.name ?? (id === 'user' ? 'You' : id)
export const boardRows = () => allRows(lanes())
