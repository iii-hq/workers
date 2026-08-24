/**
 * Live palette sources: a worker answers a query with rows, the way an
 * editor's quick open lists files as you type.
 *
 * A source is registered by a worker's setup script through
 * `host.palette.registerSource`, rides the same teardown as its other
 * registrations, and therefore exists only while the worker is connected.
 * The palette asks every source that fits the current mode, debounced,
 * with an abort signal for the query it has moved past.
 */

import { useSyncExternalStore } from 'react'
import type { PaletteSource, PaletteSourceRow } from '@/types/injectable-ui'
import type { PaletteEntry, PaletteKind } from './sources'

export interface RegisteredPaletteSource {
  /** `${scope}.${source.id}`. */
  key: string
  scope: string
  source: PaletteSource
}

let snapshot: readonly RegisteredPaletteSource[] = []
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of [...listeners]) listener()
}

export function registerPaletteSource(
  scope: string,
  source: PaletteSource,
): () => void {
  const entry: RegisteredPaletteSource = {
    key: `${scope}.${source.id}`,
    scope,
    source,
  }
  const shadowed = snapshot.find((existing) => existing.key === entry.key)
  if (shadowed) {
    console.warn(
      `[iii-ui] palette source '${entry.key}' registered twice; the newer one wins`,
    )
  }
  snapshot = [...snapshot.filter((existing) => existing !== shadowed), entry]
  emit()
  let removed = false
  return () => {
    if (removed) return
    removed = true
    snapshot = snapshot.filter((existing) => existing !== entry)
    emit()
  }
}

export function getPaletteSources(): readonly RegisteredPaletteSource[] {
  return snapshot
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

const EMPTY: readonly RegisteredPaletteSource[] = []

export function usePaletteSources(): readonly RegisteredPaletteSource[] {
  return useSyncExternalStore(subscribe, getPaletteSources, () => EMPTY)
}

export interface SourceSearchInput {
  text: string
  /** The prefix the query was typed with, if any: `#`, `>`, `/`, `@`. */
  prefix: string | null
  kinds: ReadonlySet<PaletteKind> | null
  workingDir: string | null
  conversationId: string | null
  signal: AbortSignal
}

/**
 * Ask the sources that fit: the kind must be in the mode, and the query
 * must be long enough unless it was typed with the source's own prefix. A
 * source that throws contributes nothing; the others still answer.
 */
export async function searchPaletteSources(
  sources: readonly RegisteredPaletteSource[],
  input: SourceSearchInput,
): Promise<PaletteEntry[]> {
  const asked = sources.filter(({ source }) => {
    if (input.kinds && !input.kinds.has(source.kind)) return false
    if (input.prefix !== null && input.prefix === source.prefix) return true
    return input.text.length >= (source.minQuery ?? 1)
  })
  // A source that ignores its abort signal must not hold the whole answer
  // hostage: the race ends the aggregation the moment the query is stale.
  const settled = await Promise.race([
    Promise.allSettled(
      asked.map(async ({ key, source }) => {
        const rows = await source.search(input.text, {
          workingDir: input.workingDir,
          conversationId: input.conversationId,
          signal: input.signal,
        })
        return rows.map((row) => toEntry(key, source, row))
      }),
    ),
    new Promise<null>((resolve) => {
      if (input.signal.aborted) {
        resolve(null)
        return
      }
      input.signal.addEventListener('abort', () => resolve(null), {
        once: true,
      })
    }),
  ])
  if (settled === null || input.signal.aborted) return []
  return settled.flatMap((result) =>
    result.status === 'fulfilled' ? result.value : [],
  )
}

function toEntry(
  key: string,
  source: PaletteSource,
  row: PaletteSourceRow,
): PaletteEntry {
  return {
    id: `source:${key}:${row.id}`,
    kind: source.kind,
    title: row.title,
    detail: row.detail,
    meta: row.meta ?? source.title,
    keywords: row.keywords ? [...row.keywords] : undefined,
    run: row.run,
  }
}

/** Tests only. */
export function resetPaletteSources(): void {
  snapshot = []
  emit()
}
