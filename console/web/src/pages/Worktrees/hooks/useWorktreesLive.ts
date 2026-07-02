import { useCallback, useEffect, useState } from 'react'
import { useWorktreeLifecycleEvents } from '@/hooks/use-worktree-events'
import { listWorktrees, type WorktreeInfo } from '@/lib/worktrees'

/**
 * Live registry feed for the worktrees page: `worktree::list` with status,
 * re-read on every one of the six lifecycle trigger types. While the event
 * bindings are unavailable (SDK hiccup, races around worker restart) a
 * modest poll keeps the graph honest — skipped entirely while the tab is
 * hidden so a backgrounded console never hammers the engine.
 */

export const WORKTREES_POLL_MS = 15_000

export interface WorktreesLive {
  worktrees: WorktreeInfo[]
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useWorktreesLive(enabled: boolean): WorktreesLive {
  const [worktrees, setWorktrees] = useState<WorktreeInfo[]>([])
  const [loading, setLoading] = useState(enabled)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  const { bound } = useWorktreeLifecycleEvents({
    enabled,
    onEvent: refresh,
  })

  // biome-ignore lint/correctness/useExhaustiveDependencies: token is a re-run token (bumped by events, polling, and manual refresh), not read by the effect body
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      return
    }
    // Reset loading on every fetch — the initial load, a re-enable, and each
    // token-driven refetch (poll / event / manual refresh) all go through
    // here, so the indicator reflects an in-flight refresh consistently.
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const next = await listWorktrees()
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
  }, [enabled, token])

  useEffect(() => {
    if (!enabled || bound) return
    const id = window.setInterval(() => {
      if (document.hidden) return
      refresh()
    }, WORKTREES_POLL_MS)
    return () => window.clearInterval(id)
  }, [enabled, bound, refresh])

  return { worktrees, loading, error, live: bound, refresh }
}
