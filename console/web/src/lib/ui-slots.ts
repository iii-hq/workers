/**
 * Slot registries for injectable UI — plain versioned arrays read with
 * `useSyncExternalStore`, so host components re-render exactly when a
 * script (re)registers. Registration happens through the loader's
 * per-script `host` (lib/ui-loader.tsx); components consume the `use*`
 * hooks.
 */

import { useSyncExternalStore } from 'react'
import type {
  ConfigFormProps,
  FunctionTriggerRenderer,
  PageRegistration,
  SessionChipRegistration,
} from '@/types/injectable-ui'

export interface RegisteredPage extends PageRegistration {
  /** First segment of the owning script's path — the `data-iii-ui` scope. */
  scope: string
  /** The owning script's asset path. */
  path: string
}

export interface RegisteredRenderer {
  renderer: FunctionTriggerRenderer
  scope: string
  path: string
}

export interface RegisteredConfigForm {
  /** The configuration id this form overrides (exact match). */
  configurationId: string
  /** Pre-wrapped by the loader (scope element + ErrorBoundary). */
  component: React.ComponentType<ConfigFormProps>
  scope: string
  path: string
}

export interface RegisteredSessionChip extends SessionChipRegistration {
  scope: string
  path: string
}

interface Store<T> {
  subscribe(listener: () => void): () => void
  get(): readonly T[]
  /** Append; returns a remover for exactly this entry. */
  add(entry: T): () => void
}

function createStore<T>(): Store<T> {
  let snapshot: readonly T[] = []
  const listeners = new Set<() => void>()
  const emit = () => {
    for (const l of [...listeners]) l()
  }
  return {
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    get() {
      return snapshot
    },
    add(entry) {
      snapshot = [...snapshot, entry]
      emit()
      let removed = false
      return () => {
        if (removed) return
        removed = true
        snapshot = snapshot.filter((e) => e !== entry)
        emit()
      }
    },
  }
}

const pagesStore = createStore<RegisteredPage>()
const renderersStore = createStore<RegisteredRenderer>()
const configFormsStore = createStore<RegisteredConfigForm>()
const sessionChipsStore = createStore<RegisteredSessionChip>()

/**
 * Register an extension page. Duplicate `id`: last registration wins in
 * `getExtPage` lookups (a `console.warn` names both paths); entries render
 * in registration order in the nav.
 */
export function registerExtPage(entry: RegisteredPage): () => void {
  const duplicate = pagesStore.get().find((p) => p.id === entry.id)
  if (duplicate) {
    console.warn(
      `[iii-ui] duplicate extension page id '${entry.id}' — ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return pagesStore.add(entry)
}

export function registerExtRenderer(entry: RegisteredRenderer): () => void {
  return renderersStore.add(entry)
}

/** Duplicate configuration id: last registration wins in lookups. */
export function registerExtConfigForm(entry: RegisteredConfigForm): () => void {
  const duplicate = configFormsStore
    .get()
    .find((f) => f.configurationId === entry.configurationId)
  if (duplicate) {
    console.warn(
      `[iii-ui] duplicate config-form override for '${entry.configurationId}' — ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return configFormsStore.add(entry)
}

/** Duplicate chip id: last registration wins in `useExtSessionChips`. */
export function registerExtSessionChip(
  entry: RegisteredSessionChip,
): () => void {
  const duplicate = sessionChipsStore.get().find((c) => c.id === entry.id)
  if (duplicate) {
    console.warn(
      `[iii-ui] duplicate session chip id '${entry.id}' — ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return sessionChipsStore.add(entry)
}

export function getExtPages(): readonly RegisteredPage[] {
  return pagesStore.get()
}

/** Last registration wins for duplicate ids. */
export function getExtPage(id: string): RegisteredPage | undefined {
  const pages = pagesStore.get()
  for (let i = pages.length - 1; i >= 0; i--) {
    if (pages[i].id === id) return pages[i]
  }
  return undefined
}

const EMPTY: readonly never[] = []

export function useExtPages(): readonly RegisteredPage[] {
  return useSyncExternalStore(pagesStore.subscribe, pagesStore.get, () => EMPTY)
}

export function useExtRenderers(): readonly RegisteredRenderer[] {
  return useSyncExternalStore(
    renderersStore.subscribe,
    renderersStore.get,
    () => EMPTY,
  )
}

function dedupeSessionChips(
  chips: readonly RegisteredSessionChip[],
): readonly RegisteredSessionChip[] {
  const byId = new Map<string, RegisteredSessionChip>()
  for (const chip of chips) byId.set(chip.id, chip)
  return [...byId.values()]
}

/** Session chips deduplicated by id — last registration wins. */
export function getExtSessionChips(): readonly RegisteredSessionChip[] {
  return dedupeSessionChips(sessionChipsStore.get())
}

/**
 * Session chips in registration order, deduplicated by id (last
 * registration wins, matching the pages slot).
 */
export function useExtSessionChips(): readonly RegisteredSessionChip[] {
  const chips = useSyncExternalStore(
    sessionChipsStore.subscribe,
    sessionChipsStore.get,
    () => EMPTY,
  )
  return dedupeSessionChips(chips)
}

/** The injected form override for one configuration id (last wins). */
export function useExtConfigForm(
  configurationId: string,
): RegisteredConfigForm | undefined {
  const forms = useSyncExternalStore(
    configFormsStore.subscribe,
    configFormsStore.get,
    () => EMPTY,
  )
  for (let i = forms.length - 1; i >= 0; i--) {
    if (forms[i].configurationId === configurationId) return forms[i]
  }
  return undefined
}
