import { useCallback, useEffect, useState } from 'react'
import { useMemoryEvents } from '@/hooks/use-memory-events'
import {
  FACTS_PAGE_SIZE,
  listBanks,
  listBlocks,
  listFacts,
  type MemoryBank,
  type MemoryBlock,
  type MemoryFact,
} from '@/lib/memory'

/**
 * Live state for the memory page: banks + the selected bank's facts and
 * blocks, re-read on both memory trigger types (`memory::item-changed`,
 * `memory::bank-changed`). While the event bindings are unavailable a
 * modest poll keeps the page honest — skipped while the tab is hidden.
 */

export const MEMORY_POLL_MS = 15_000

export interface MemoryLive {
  banks: MemoryBank[]
  selected: string | null
  setSelected: (bank: string) => void
  facts: MemoryFact[]
  total: number
  offset: number
  setOffset: (next: number) => void
  pageSize: number
  blocks: MemoryBlock[]
  includeSuperseded: boolean
  setIncludeSuperseded: (next: boolean) => void
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useMemoryLive(enabled: boolean): MemoryLive {
  const [banks, setBanks] = useState<MemoryBank[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [facts, setFacts] = useState<MemoryFact[]>([])
  const [total, setTotal] = useState(0)
  const [offset, setOffset] = useState(0)
  const [blocks, setBlocks] = useState<MemoryBlock[]>([])
  const [includeSuperseded, setIncludeSuperseded] = useState(false)
  const [loading, setLoading] = useState(enabled)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  const selectBank = useCallback((bank: string) => {
    setOffset(0)
    setSelected(bank)
  }, [])

  const setIncludeSupersededReset = useCallback((next: boolean) => {
    setOffset(0)
    setIncludeSuperseded(next)
  }, [])

  const { bound } = useMemoryEvents({ enabled, onEvent: refresh })

  // biome-ignore lint/correctness/useExhaustiveDependencies: token is a re-run token (bumped by events, polling, and manual refresh), not read by the effect body
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      return
    }
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const nextBanks = await listBanks()
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
          const [factPage, nextBlocks] = await Promise.all([
            listFacts(bank, includeSuperseded, offset),
            listBlocks(bank),
          ])
          if (cancelled) return
          setFacts(factPage.facts)
          setTotal(factPage.total)
          setBlocks(nextBlocks)
        } else {
          setFacts([])
          setTotal(0)
          setBlocks([])
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
  }, [enabled, token, selected, includeSuperseded, offset])

  useEffect(() => {
    if (!enabled || bound) return
    const id = window.setInterval(() => {
      if (document.hidden) return
      refresh()
    }, MEMORY_POLL_MS)
    return () => window.clearInterval(id)
  }, [enabled, bound, refresh])

  return {
    banks,
    selected,
    setSelected: selectBank,
    facts,
    total,
    offset,
    setOffset,
    pageSize: FACTS_PAGE_SIZE,
    blocks,
    includeSuperseded,
    setIncludeSuperseded: setIncludeSupersededReset,
    loading,
    error,
    live: bound,
    refresh,
  }
}
