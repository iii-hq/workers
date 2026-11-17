import type { ExtensionIii } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type AgentSession, type SessionRow, toAgentSession } from './agent-tree'

/** One `session::list` page; the store clamps larger asks to its own maximum. */
const PAGE = 200
/** Pages read at most: the newest 1,000 sessions cover every live run, and the tree never pages on its own. */
const MAX_PAGES = 5
/** session-manager's events: a session started, changed state, was renamed or went away. */
const EVENTS = ['session::on-created', 'session::on-status-changed', 'session::on-meta-updated', 'session::on-deleted']

/** Each mounted tree registers its own handler: two panes must not share one. */
let instances = 0

type ListResponse = { sessions?: SessionRow[]; next_cursor?: string | null }

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

export type AgentSessions = {
  sessions: AgentSession[]
  /** True until the first read settles. */
  loading: boolean
  /** Why the sessions could not be read (session-manager missing or down); the tickets still show. */
  error: string | null
  reload: () => void
}

/**
 * The agent sessions, kept live while the tree is on screen: session-manager's events schedule a quiet re-read
 * (a burst of them is one read), and so does coming back to the tab.
 */
export function useAgentSessions(iii: ExtensionIii, enabled: boolean): AgentSessions {
  const [sessions, setSessions] = useState<AgentSession[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const seq = useRef(0)
  const timer = useRef<number | undefined>(undefined)

  const read = useCallback(async () => {
    const token = ++seq.current
    try {
      const next = await readSessions(iii)
      if (token !== seq.current) return
      setSessions(next)
      setError(null)
    } catch (caught) {
      if (token !== seq.current) return
      setError(errorMessage(caught))
    } finally {
      if (token === seq.current) setLoading(false)
    }
  }, [iii])

  const schedule = useCallback(() => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => void read(), 400)
  }, [read])

  useEffect(() => {
    if (!enabled) return
    void read()
    const handlerId = `iii::kanban-ui::sessions-${++instances}`
    const offHandler = iii.on(handlerId, () => schedule())
    const offObservers = EVENTS.map((fn) => iii.observe(fn, `${handlerId}::${iii.browserId}`))
    const onVisible = () => {
      if (document.visibilityState === 'visible') schedule()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      for (const off of offObservers) off()
      offHandler()
      window.clearTimeout(timer.current)
      seq.current += 1
    }
  }, [enabled, iii, read, schedule])

  return { sessions, loading, error, reload: read }
}
