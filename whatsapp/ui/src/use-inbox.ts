import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { Coverage } from './inbox-model'

interface ArchivePage<T> { items: T[]; next_cursor: string | null; coverage: Coverage }
interface ArchiveState<T> {
  items: T[]
  nextCursor: string | null
  coverage: Coverage | null
  loading: boolean
  loadingMore: boolean
  error: string | null
}

/** Serial page requests, latest-page refresh, and disposal on account/chat changes. */
export function useInboxPages<T>({ identity, fetchPage, merge, pollMs = 5000, enabled = true }: {
  identity: string
  fetchPage(cursor?: string): Promise<ArchivePage<T>>
  merge(previous: T[], incoming: T[]): T[]
  pollMs?: number
  enabled?: boolean
}) {
  const [state, setState] = useState<ArchiveState<T>>({ items: [], nextCursor: null, coverage: null, loading: true, loadingMore: false, error: null })
  const operations = useRef({ refresh: () => {}, more: () => {} })
  const callbacks = useRef({ fetchPage, merge })
  callbacks.current = { fetchPage, merge }
  const refresh = useCallback(() => operations.current.refresh(), [])
  const more = useCallback(() => operations.current.more(), [])

  useEffect(() => {
    let active = true
    let busy = false
    let items: T[] = []
    let nextCursor: string | null = null
    let loadedPages = 1
    let loaded = false
    const api = callbacks.current
    setState({ items: [], nextCursor: null, coverage: null, loading: enabled, loadingMore: false, error: null })
    if (!enabled) {
      operations.current = { refresh: () => {}, more: () => {} }
      return
    }

    const load = async (append: boolean) => {
      if (!active || busy || (append && !nextCursor)) return
      busy = true
      setState((previous) => ({ ...previous, loading: !loaded, loadingMore: append }))
      try {
        let page = await api.fetchPage(append ? nextCursor! : undefined)
        if (!active) return
        let received = page.items
        let refreshedPages = 1
        // Re-read loaded pages instead of retaining old bodies indefinitely:
        // expiry, deletion and retention may change any previously read page.
        if (!append) {
          while (page.next_cursor && refreshedPages < loadedPages) {
            page = await api.fetchPage(page.next_cursor)
            if (!active) return
            received = [...received, ...page.items]
            refreshedPages += 1
          }
        }
        items = api.merge(append ? items : [], received)
        nextCursor = page.next_cursor
        loadedPages = append ? loadedPages + 1 : refreshedPages
        loaded = true
        setState({ items, nextCursor, coverage: page.coverage, loading: false, loadingMore: false, error: null })
      } catch (cause) {
        if (active) {
          items = []
          loaded = false
          loadedPages = 1
          nextCursor = null
          setState((previous) => ({ ...previous, items: [], nextCursor: null, loading: false, loadingMore: false, error: errorMessage(cause) }))
        }
      } finally { busy = false }
    }
    operations.current = { refresh: () => void load(false), more: () => void load(true) }
    void load(false)
    const visibleRefresh = () => { if (!document.hidden) void load(false) }
    const timer = window.setInterval(visibleRefresh, pollMs)
    document.addEventListener('visibilitychange', visibleRefresh)
    return () => {
      active = false
      operations.current = { refresh: () => {}, more: () => {} }
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', visibleRefresh)
    }
  }, [identity, pollMs, enabled])
  return { ...state, refresh, more }
}
