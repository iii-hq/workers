import type { ExtensionIii } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import { subscribeChanges } from '../shared'
import { type AgentSession, type SessionRow, type TreeLink, toAgentSession } from './agent-tree'

/** One `session::list` page; the store clamps larger asks to its own maximum. */
const PAGE = 200
/** Pages read at most: the newest 1,000 sessions cover every live run, and the tree never pages on its own. */
const MAX_PAGES = 5
/** session-manager's events: a session started, changed state, was renamed or went away. */
const STATE_EVENTS = [
  'session::on-created',
  'session::on-status-changed',
  'session::on-meta-updated',
  'session::on-deleted',
]
/** A message landed in a session: a prompt reaching an agent, or its reply. */
const MESSAGE_EVENT = 'session::on-message-added'
/** Reads at most this often while events keep coming (a burst is one read, a stream is one a second). */
const MIN_READ_GAP_MS = 1000

/** Each mounted tree registers its own handlers: two panes must not share one. */
let instances = 0

type ListResponse = { sessions?: SessionRow[]; next_cursor?: string | null }
type LinksResponse = { links?: TreeLink[] }
type MessageAdded = { session_id?: string; message?: { role?: string }; timestamp?: number }

/** Every session a person or an automation runs (e2e suites stay out), newest first. */
async function readSessions(iii: ExtensionIii): Promise<AgentSession[]> {
  const rows: SessionRow[] = []
  let cursor: string | null | undefined
  for (let page = 0; page < MAX_PAGES; page += 1) {
    const reply = await iii.call<ListResponse>(
      'session::list',
      { limit: PAGE, order: 'updated_desc', kinds: ['user', 'automation'], ...(cursor ? { cursor } : {}) },
      { timeoutMs: 10_000 },
    )
    rows.push(...(reply?.sessions ?? []))
    cursor = reply?.next_cursor
    if (!cursor) break
  }
  return rows.map(toAgentSession)
}

/**
 * The ticket ↔ session links kanban recorded from each change's own session. `null` when this kanban predates
 * them (the tree then places sessions by their names, as before).
 */
async function readLinks(iii: ExtensionIii, projectId: string | null): Promise<TreeLink[] | null> {
  try {
    const reply = await iii.call<LinksResponse>('kanban::sessions::links', projectId ? { project_id: projectId } : {}, {
      timeoutMs: 10_000,
    })
    return reply?.links ?? []
  } catch {
    return null
  }
}

export type AgentSessions = {
  sessions: AgentSession[]
  /** Exact links, or `null` on a kanban without them. */
  links: TreeLink[] | null
  /** When a prompt (a user-role message: a person's, or a parent agent's task) last reached each session. */
  prompted: ReadonlyMap<string, number>
  /** True until the first read settles. */
  loading: boolean
  /** Why the sessions could not be read (session-manager missing or down); the tickets still show. */
  error: string | null
  reload: () => void
}

/**
 * The agent sessions and their ticket links, kept live while the tree is on screen. session-manager's and
 * kanban's events schedule a quiet re-read — at most one a second while they keep coming, so a run streaming
 * events is read steadily instead of never — and so does coming back to the tab.
 */
export function useAgentSessions(iii: ExtensionIii, enabled: boolean, projectId: string | null): AgentSessions {
  const [sessions, setSessions] = useState<AgentSession[]>([])
  const [links, setLinks] = useState<TreeLink[] | null>(null)
  const [prompted, setPrompted] = useState<ReadonlyMap<string, number>>(new Map())
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const seq = useRef(0)
  const timer = useRef<number | undefined>(undefined)
  const lastRead = useRef(0)

  const read = useCallback(async () => {
    const token = ++seq.current
    lastRead.current = Date.now()
    try {
      const [next, nextLinks] = await Promise.all([readSessions(iii), readLinks(iii, projectId)])
      if (token !== seq.current) return
      setSessions(next)
      setLinks(nextLinks)
      setError(null)
    } catch (caught) {
      if (token !== seq.current) return
      setError(errorMessage(caught))
    } finally {
      if (token === seq.current) setLoading(false)
    }
  }, [iii, projectId])

  /** Throttled: the first event reads soon, a stream reads once a second, the last one is never lost. */
  const schedule = useCallback(() => {
    if (timer.current !== undefined) return
    const wait = Math.max(150, MIN_READ_GAP_MS - (Date.now() - lastRead.current))
    timer.current = window.setTimeout(() => {
      timer.current = undefined
      void read()
    }, wait)
  }, [read])

  useEffect(() => {
    if (!enabled) return
    void read()
    const n = ++instances
    const stateHandler = `iii::kanban-ui::sessions-${n}`
    const messageHandler = `iii::kanban-ui::messages-${n}`
    const offState = iii.on(stateHandler, () => schedule())
    const offMessage = iii.on<MessageAdded>(messageHandler, (event) => {
      if (event?.message?.role === 'user' && event.session_id) {
        const id = event.session_id
        setPrompted((current) => new Map(current).set(id, Date.now()))
      }
      schedule()
    })
    const offObservers = [
      ...STATE_EVENTS.map((fn) => iii.observe(fn, `${stateHandler}::${iii.browserId}`)),
      iii.observe(MESSAGE_EVENT, `${messageHandler}::${iii.browserId}`),
    ]
    // A ticket write may carry a new link (its activity names the session that made it).
    const offKanban = subscribeChanges(iii, () => schedule())
    const onVisible = () => {
      if (document.visibilityState === 'visible') schedule()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      for (const off of offObservers) off()
      offState()
      offMessage()
      offKanban()
      window.clearTimeout(timer.current)
      timer.current = undefined
      seq.current += 1
    }
  }, [enabled, iii, read, schedule])

  return { sessions, links, prompted, loading, error, reload: read }
}
