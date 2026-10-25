/**
 * The quick menu's five models.
 *
 * The composer's model menu shows a short, ordered list the person keeps
 * (the reference: five models, drag to reorder, an Edit dialog to swap them)
 * rather than the whole catalog. This module is that list: what is stored,
 * how a stored list is reconciled with the catalog that is actually loaded,
 * the pure edits (pin, unpin, move, effort), and the hook the menu reads it
 * through. Nothing here touches the catalog itself.
 *
 * The first slot is the default: a new chat starts on it (see `loadLastModel`
 * in `lib/storage`), which is why the dialog badges it.
 */

import { useSyncExternalStore } from 'react'
import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
  type ThinkingLevel,
} from '@/types/chat'

export interface PinnedModel {
  id: ModelId
  /** The effort the pin applies when picked; `default` leaves the model's own. */
  effort: ThinkingLevel
}

export const PINNED_MODELS_MAX = 5

const STORAGE_KEY = 'iii-chat-pinned-models'
const CHANGE_EVENT = 'iii-pinned-models-change'

function isPinnedModel(value: unknown): value is PinnedModel {
  if (!value || typeof value !== 'object') return false
  const row = value as Record<string, unknown>
  return (
    typeof row.id === 'string' &&
    row.id.length > 0 &&
    typeof row.effort === 'string' &&
    row.effort.length > 0
  )
}

/** The stored list, or `null` when the person has never edited it. */
export function loadPinnedModels(): PinnedModel[] | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return null
    return dedupe(parsed.filter(isPinnedModel)).slice(0, PINNED_MODELS_MAX)
  } catch {
    return null
  }
}

/** Store the list (`null` forgets it); every `usePinnedModels` re-reads. */
export function savePinnedModels(list: PinnedModel[] | null): void {
  try {
    if (list === null) localStorage.removeItem(STORAGE_KEY)
    else {
      localStorage.setItem(
        STORAGE_KEY,
        JSON.stringify(dedupe(list).slice(0, PINNED_MODELS_MAX)),
      )
    }
  } catch {
    /* best-effort */
  }
  snapshot = undefined
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new Event(CHANGE_EVENT))
  }
}

/** The first slot: what a new chat starts on once the list has been edited. */
export function pinnedDefaultModel(): PinnedModel | null {
  return loadPinnedModels()?.[0] ?? null
}

// `useSyncExternalStore` wants a stable snapshot between changes, so the
// parsed list is cached until a save (or another tab's storage event).
let snapshot: PinnedModel[] | null | undefined

function readSnapshot(): PinnedModel[] | null {
  if (snapshot === undefined) snapshot = loadPinnedModels()
  return snapshot
}

function subscribe(onChange: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === null || event.key === STORAGE_KEY) {
      snapshot = undefined
      onChange()
    }
  }
  window.addEventListener(CHANGE_EVENT, onChange)
  window.addEventListener('storage', onStorage)
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange)
    window.removeEventListener('storage', onStorage)
  }
}

/** The stored list, live; `null` until the person edits it. */
export function usePinnedModels(): PinnedModel[] | null {
  return useSyncExternalStore(subscribe, readSnapshot, () => null)
}

function providerOf(id: ModelId): string {
  return id.split(CATALOG_MODEL_KEY_SEP)[0] ?? ''
}

function dedupe(list: readonly PinnedModel[]): PinnedModel[] {
  const seen = new Set<string>()
  return list.filter((entry) => {
    if (seen.has(entry.id)) return false
    seen.add(entry.id)
    return true
  })
}

/**
 * The list before anyone edits it: the selected model first, then the rest
 * of its provider's catalog in order, then the other providers', up to five.
 * Every pin at the model's own default effort.
 */
export function defaultPinnedModels(
  options: readonly ModelOption[],
  selected: ModelId | null,
): PinnedModel[] {
  const ids: ModelId[] = []
  if (selected && options.some((option) => option.id === selected)) {
    ids.push(selected)
  }
  const provider = selected ? providerOf(selected) : null
  const ranked = [...options].sort((a, b) => {
    const aSame = provider !== null && providerOf(a.id) === provider ? 0 : 1
    const bSame = provider !== null && providerOf(b.id) === provider ? 0 : 1
    return aSame - bSame
  })
  for (const option of ranked) {
    if (ids.length >= PINNED_MODELS_MAX) break
    if (!ids.includes(option.id)) ids.push(option.id)
  }
  return ids.map((id) => ({ id, effort: 'default' }))
}

/**
 * What the menu shows: the stored list with the models the catalog no
 * longer offers dropped, or the default list when nothing is stored (or
 * nothing stored survived).
 */
export function resolvePinnedModels(
  stored: readonly PinnedModel[] | null,
  options: readonly ModelOption[],
  selected: ModelId | null,
): PinnedModel[] {
  if (stored === null) return defaultPinnedModels(options, selected)
  const offered = new Set(options.map((option) => option.id))
  const kept = stored.filter((entry) => offered.has(entry.id))
  if (kept.length === 0 && stored.length > 0 && options.length > 0) {
    return defaultPinnedModels(options, selected)
  }
  return kept
}

/**
 * Pin a model, at `at` or at the end. An already pinned model moves rather
 * than doubling; when the list is full and the model is new, the last pin
 * makes room. Effort is kept for a model already pinned unless given.
 */
export function pinModel(
  list: readonly PinnedModel[],
  entry: PinnedModel,
  at?: number,
): PinnedModel[] {
  const existing = list.find((pinned) => pinned.id === entry.id)
  const without = list.filter((pinned) => pinned.id !== entry.id)
  const next = existing ? { ...existing, effort: entry.effort } : entry
  if (!existing && without.length >= PINNED_MODELS_MAX) without.pop()
  const index =
    at === undefined
      ? without.length
      : Math.max(0, Math.min(at, without.length))
  return [...without.slice(0, index), next, ...without.slice(index)]
}

export function unpinModel(
  list: readonly PinnedModel[],
  id: ModelId,
): PinnedModel[] {
  return list.filter((pinned) => pinned.id !== id)
}

/** Move a pinned model to `toIndex` (clamped); a model not pinned is left alone. */
export function movePinnedModel(
  list: readonly PinnedModel[],
  id: ModelId,
  toIndex: number,
): PinnedModel[] {
  const entry = list.find((pinned) => pinned.id === id)
  if (!entry) return [...list]
  const without = list.filter((pinned) => pinned.id !== id)
  const index = Math.max(0, Math.min(toIndex, without.length))
  return [...without.slice(0, index), entry, ...without.slice(index)]
}

/** The pin's effort, for the model that is pinned; others are untouched. */
export function withPinnedEffort(
  list: readonly PinnedModel[],
  id: ModelId,
  effort: ThinkingLevel,
): PinnedModel[] {
  return list.map((pinned) =>
    pinned.id === id && pinned.effort !== effort
      ? { ...pinned, effort }
      : pinned,
  )
}

export function isPinned(list: readonly PinnedModel[], id: ModelId): boolean {
  return list.some((pinned) => pinned.id === id)
}
