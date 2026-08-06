import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  listWorktrees,
  WORKTREE_LIFECYCLE_TRIGGERS,
  type WorktreeInfo,
} from './worktree-data'

/**
 * Live registry feed for the worktrees page: `worktree::list` with status,
 * re-read on every one of the six lifecycle trigger types. Each binding is a
 * tab-scoped Message-path trigger — `host.iii.registerTrigger` targeting an
 * `iii::worktree-ui::<trigger>::<browserId>` handler (the `iii::` prefix keeps
 * the per-event invocations span-suppressed and out of the trace feed), GC'd
 * with the tab. While the bindings are unavailable (SDK hiccup, races around
 * worker restart) a modest poll keeps the graph honest — skipped while the tab
 * is hidden so a backgrounded console never hammers the engine.
 */

export const WORKTREES_POLL_MS = 15_000

/** Per-tab handler prefix (host.iii namespaces it `::<browserId>`). */
const EVENTS_FN = 'iii::worktree-ui::events'

export interface WorktreesLive {
  worktrees: WorktreeInfo[]
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useWorktreesLive(host: Host): WorktreesLive {
  const [worktrees, setWorktrees] = useState<WorktreeInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [bound, setBound] = useState(false)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])
  const refreshRef = useRef(refresh)
  refreshRef.current = refresh

  // Bind all six lifecycle trigger types; any event re-reads the registry.
  useEffect(() => {
    const offs: Array<() => void> = []
    let registered = 0
    for (const triggerType of WORKTREE_LIFECYCLE_TRIGGERS) {
      const slug = triggerType.replace(/[^a-z0-9]+/g, '-')
      const fnId = `${EVENTS_FN}::${slug}::${host.iii.browserId}`
      try {
        offs.push(
          host.iii.on(fnId, () => {
            refreshRef.current()
          }),
        )
        offs.push(
          host.iii.registerTrigger({
            type: triggerType,
            function_id: fnId,
            config: {},
          }),
        )
        registered += 1
      } catch {
        // Worker absent or trigger type unregistered; drop the binding and
        // let the poll below keep the graph fresh.
      }
    }
    setBound(registered === WORKTREE_LIFECYCLE_TRIGGERS.length)
    return () => {
      setBound(false)
      for (const off of offs) off()
    }
  }, [host])

  // Reset loading on every fetch — the initial load and each token-driven
  // refetch (poll / event / manual refresh) all go through here, so the
  // indicator reflects an in-flight refresh consistently.
  // biome-ignore lint/correctness/useExhaustiveDependencies: token is a re-run token (bumped by events, polling, and manual refresh), not read by the effect body
  useEffect(() => {
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const next = await listWorktrees(host)
        if (cancelled) return
        setWorktrees(next)
        setError(null)
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [host, token])

  useEffect(() => {
    if (bound) return
    const id = window.setInterval(() => {
      if (document.hidden) return
      refreshRef.current()
    }, WORKTREES_POLL_MS)
    return () => window.clearInterval(id)
  }, [bound])

  return { worktrees, loading, error, live: bound, refresh }
}
