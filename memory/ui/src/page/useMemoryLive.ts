import type { Host } from '@iii-dev/console-ui'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useCallback, useEffect, useRef, useState } from 'react'
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
 * (`memory::item-changed`, `memory::bank-changed`) through the shared
 * `useWorkerLive` (tab-scoped bindings, visible-tab poll while they are
 * unavailable). The selection/page/filter live here so a change re-runs the
 * fetch; the function ids and trigger types are verbatim from the console.
 */

/** Per-tab handler id (the host namespaces it `::<browserId>`). */
const EVENTS_FN = 'iii::memory-ui::events'
const MEMORY_EVENT_TRIGGERS = [
  'memory::item-changed',
  'memory::bank-changed',
] as const

interface MemorySnapshot {
  banks: MemoryBank[]
  memories: MemoryItem[]
  total: number
  rules: MemoryRule[]
  tags: { tag: string; count: number }[]
}

const EMPTY: MemorySnapshot = {
  banks: [],
  memories: [],
  total: 0,
  rules: [],
  tags: [],
}

export interface MemoryLive extends MemorySnapshot {
  selected: string | null
  setSelected: (bank: string) => void
  offset: number
  setOffset: (next: number) => void
  pageSize: number
  includeSuperseded: boolean
  setIncludeSuperseded: (next: boolean) => void
  /** Active tag filter (null = all). */
  tag: string | null
  setTag: (next: string | null) => void
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useMemoryLive(host: Host): MemoryLive {
  const [selected, setSelected] = useState<string | null>(null)
  const [offset, setOffset] = useState(0)
  const [includeSuperseded, setIncludeSuperseded] = useState(false)
  const [tag, setTag] = useState<string | null>(null)

  const { data, loading, error, live, refresh } = useWorkerLive<MemorySnapshot>({
    iii: host.iii,
    triggers: MEMORY_EVENT_TRIGGERS,
    handlerId: EVENTS_FN,
    fetch: async () => {
      const banks = await listBanks(host)
      // Keep the selection stable across refreshes; adopt a sensible
      // default on first load (prefer `main`, else the first bank).
      let bank = selected
      if (!bank || !banks.some((b) => b.name === bank)) {
        bank =
          banks.find((b) => b.name === 'main')?.name ?? banks[0]?.name ?? null
        setSelected(bank)
      }
      if (!bank) return { ...EMPTY, banks }
      const [page, rules, tags] = await Promise.all([
        listMemories(host, bank, includeSuperseded, offset, MEMORIES_PAGE_SIZE, tag),
        listRules(host, bank),
        listTags(host, bank),
      ])
      return { banks, memories: page.memories, total: page.total, rules, tags }
    },
  })

  // A selection/page/filter change re-runs the fetch (the mount fetch is
  // the hook's own).
  const mounted = useRef(false)
  // biome-ignore lint/correctness/useExhaustiveDependencies: the deps are the fetch inputs, not read here
  useEffect(() => {
    if (mounted.current) refresh()
    mounted.current = true
  }, [selected, includeSuperseded, offset, tag, refresh])

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

  return {
    ...(data ?? EMPTY),
    selected,
    setSelected: selectBank,
    offset,
    setOffset,
    pageSize: MEMORIES_PAGE_SIZE,
    includeSuperseded,
    setIncludeSuperseded: setIncludeSupersededReset,
    tag,
    setTag: setTagReset,
    loading,
    error,
    live,
    refresh,
  }
}
