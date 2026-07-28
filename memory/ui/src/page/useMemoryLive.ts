import { useCallback, useEffect, useState } from 'react'
import type { Host } from '@iii-dev/console-ui'
import {
  MEMORIES_PAGE_SIZE,
  listBanks,
  listMemories,
  listRules,
  listTags,
  type MemoryBank,
  type MemoryItem,
  type MemoryRule,
} from './memory-data'

/**
 * Live state for the memory page: banks + the selected bank's memories and
 * rules, re-read on both of the memory worker's trigger types
 * (`memory::item-changed`, `memory::bank-changed`). One tab-scoped binding
 * per type is registered over `host.iii` — same pattern as the state page's
 * event hub and iii-directory's `useOnChange`: an `iii::`-prefixed handler id
 * (span-suppressed) whose trigger is GC'd with the tab. While the bindings
 * are unavailable a modest poll keeps the page honest — skipped while the tab
 * is hidden.
 *
 * Ported from the console's `useMemoryLive` + `useMemoryEvents`; the function
 * ids and trigger types are verbatim, only the transport is the injected
 * `host.iii`.
 */

export const MEMORY_POLL_MS = 15_000

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
const EVENTS_FN = 'iii::memory-ui::events'
const MEMORY_EVENT_TRIGGERS = [
  'memory::item-changed',
  'memory::bank-changed',
] as const

export interface MemoryLive {
  banks: MemoryBank[]
  selected: string | null
  setSelected: (bank: string) => void
  memories: MemoryItem[]
  total: number
  offset: number
  setOffset: (next: number) => void
  pageSize: number
  rules: MemoryRule[]
  includeSuperseded: boolean
  setIncludeSuperseded: (next: boolean) => void
  /** Active tag filter (null = all) and the bank's tag cloud. */
  tag: string | null
  setTag: (next: string | null) => void
  tags: { tag: string; count: number }[]
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useMemoryLive(host: Host): MemoryLive {
  const [banks, setBanks] = useState<MemoryBank[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [memories, setMemories] = useState<MemoryItem[]>([])
  const [total, setTotal] = useState(0)
  const [offset, setOffset] = useState(0)
  const [rules, setRules] = useState<MemoryRule[]>([])
  const [includeSuperseded, setIncludeSuperseded] = useState(false)
  const [tag, setTag] = useState<string | null>(null)
  const [tags, setTags] = useState<{ tag: string; count: number }[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [bound, setBound] = useState(false)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  const selectBank = useCallback((bank: string) => {
    setOffset(0)
    setTag(null)
    setSelected(bank)
  }, [])

  const setTagReset = useCallback((next: string | null) => {
    setOffset(0)
    setTag(next)
  }, [])

  const setIncludeSupersededReset = useCallback((next: boolean) => {
    setOffset(0)
    setIncludeSuperseded(next)
  }, [])

  // Tab-scoped live bindings: one trigger per memory event type, both routed
  // to the same span-suppressed handler; every event refreshes the page.
  useEffect(() => {
    const offHandler = host.iii.on(EVENTS_FN, () => refresh())
    const offTriggers = MEMORY_EVENT_TRIGGERS.map((type) =>
      host.iii.registerTrigger({
        type,
        function_id: `${EVENTS_FN}::${host.iii.browserId}`,
        config: {},
      }),
    )
    setBound(true)
    return () => {
      setBound(false)
      for (const off of offTriggers) off()
      offHandler()
    }
  }, [host, refresh])

  // biome-ignore lint/correctness/useExhaustiveDependencies: token is a re-run token (bumped by events, polling, and manual refresh), not read by the effect body
  useEffect(() => {
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const nextBanks = await listBanks(host)
        if (cancelled) return
        setBanks(nextBanks)
        // Keep the selection stable across refreshes; adopt a sensible
        // default on first load (prefer `main`, else the first bank).
        let bank = selected
        if (!bank || !nextBanks.some((b) => b.name === bank)) {
          bank =
            nextBanks.find((b) => b.name === 'main')?.name ??
            nextBanks[0]?.name ??
            null
          setSelected(bank)
        }
        if (bank) {
          const [memoryPage, nextRules, nextTags] = await Promise.all([
            listMemories(
              host,
              bank,
              includeSuperseded,
              offset,
              MEMORIES_PAGE_SIZE,
              tag,
            ),
            listRules(host, bank),
            listTags(host, bank),
          ])
          if (cancelled) return
          setMemories(memoryPage.memories)
          setTotal(memoryPage.total)
          setRules(nextRules)
          setTags(nextTags)
        } else {
          setMemories([])
          setTotal(0)
          setRules([])
        }
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
  }, [host, token, selected, includeSuperseded, offset, tag])

  useEffect(() => {
    if (bound) return
    const id = window.setInterval(() => {
      if (document.hidden) return
      refresh()
    }, MEMORY_POLL_MS)
    return () => window.clearInterval(id)
  }, [bound, refresh])

  return {
    banks,
    selected,
    setSelected: selectBank,
    memories,
    total,
    offset,
    setOffset,
    pageSize: MEMORIES_PAGE_SIZE,
    rules,
    includeSuperseded,
    setIncludeSuperseded: setIncludeSupersededReset,
    tag,
    setTag: setTagReset,
    tags,
    loading,
    error,
    live: bound,
    refresh,
  }
}
