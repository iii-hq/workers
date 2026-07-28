import { useCallback, useEffect, useState } from 'react'

/**
 * One in-flight github read: run `fetcher` while `enabled`, re-run when the
 * fetcher identity changes (callers memo it on repo/filter deps) or on
 * `refresh`. No polling and no live trigger bindings on purpose — the data
 * changes on GitHub's side, not the engine's, so the page refreshes on demand
 * instead of hammering the gh rate limit.
 */

export interface GithubRead<T> {
  data: T | null
  loading: boolean
  error: string | null
  refresh: () => void
}

export function useGithubRead<T>(
  enabled: boolean,
  fetcher: () => Promise<T>,
): GithubRead<T> {
  const [data, setData] = useState<T | null>(null)
  const [loading, setLoading] = useState(enabled)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  // `token` is a re-run token (bumped by manual refresh), not read by the
  // effect body — it only needs to be in the dependency list.
  useEffect(() => {
    if (!enabled) {
      setData(null)
      setLoading(false)
      setError(null)
      return
    }
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const next = await fetcher()
        if (cancelled) return
        setData(next)
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
  }, [enabled, fetcher, token])

  return { data, loading, error, refresh }
}
