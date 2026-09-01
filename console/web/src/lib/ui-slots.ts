/**
 * Slot registries for injectable UI — plain versioned arrays read with
 * `useSyncExternalStore`, so host components re-render exactly when a
 * script (re)registers. Registration happens through the loader's
 * per-script `host` (lib/ui-loader.tsx); components consume the `use*`
 * hooks.
 */

import { useMemo, useSyncExternalStore } from 'react'
import type {
  ConfigFormLayout,
  ConfigFormProps,
  FunctionTriggerRenderer,
  PageRegistration,
  ProviderConfigFormProps,
  SessionChipRegistration,
  SessionTurnSummaryRegistration,
  TriggerActivityRenderer,
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

export interface RegisteredTriggerActivityRenderer {
  renderer: TriggerActivityRenderer
  scope: string
  path: string
}

export interface RegisteredConfigForm {
  /** The configuration id this form overrides (exact match). */
  configurationId: string
  /** Pre-wrapped by the loader (scope element + ErrorBoundary). */
  component: React.ComponentType<ConfigFormProps>
  /** Host-owned layout for the form region. */
  layout: ConfigFormLayout
  scope: string
  path: string
}

export interface RegisteredProviderConfigForm {
  /** Exact llm-router provider id this form overrides. */
  providerId: string
  /** Pre-wrapped by the loader (scope element + ErrorBoundary). */
  component: React.ComponentType<ProviderConfigFormProps>
  scope: string
  path: string
}

export interface RegisteredSessionChip extends SessionChipRegistration {
  scope: string
  path: string
}

export interface RegisteredSessionTurnSummary
  extends SessionTurnSummaryRegistration {
  scope: string
  path: string
}

interface Store<T> {
  subscribe(listener: () => void): () => void
  get(): readonly T[]
  /** Append; returns a remover for exactly this entry. */
  add(entry: T): () => void
}

interface ValueStore<T> {
  subscribe(listener: () => void): () => void
  get(): T
  set(value: T): void
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

function createValueStore<T>(initialValue: T): ValueStore<T> {
  let value = initialValue
  const listeners = new Set<() => void>()
  return {
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    get() {
      return value
    },
    set(nextValue) {
      if (nextValue === value) return
      value = nextValue
      for (const listener of [...listeners]) listener()
    },
  }
}

export type UiAssetsStatus = 'loading' | 'ready' | 'unavailable'

const pagesStore = createStore<RegisteredPage>()
const renderersStore = createStore<RegisteredRenderer>()
const triggerActivityRenderersStore =
  createStore<RegisteredTriggerActivityRenderer>()
const configFormsStore = createStore<RegisteredConfigForm>()
const providerConfigFormsStore = createStore<RegisteredProviderConfigForm>()
const sessionChipsStore = createStore<RegisteredSessionChip>()
const sessionTurnSummariesStore = createStore<RegisteredSessionTurnSummary>()
const uiAssetsStatusStore = createValueStore<UiAssetsStatus>('unavailable')

/**
 * Register an extension page. Duplicate `id`: last registration wins in
 * `getExtPage` lookups (a `console.warn` names both paths); entries render
 * in registration order in the nav.
 */
export function registerExtPage(entry: RegisteredPage): () => void {
  const duplicate = pagesStore.get().find((p) => p.id === entry.id)
  if (duplicate && duplicate.path !== entry.path) {
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

export function registerExtTriggerActivityRenderer(
  entry: RegisteredTriggerActivityRenderer,
): () => void {
  return triggerActivityRenderersStore.add(entry)
}

/** Duplicate configuration id: last registration wins in lookups. */
export function registerExtConfigForm(entry: RegisteredConfigForm): () => void {
  const duplicate = configFormsStore
    .get()
    .find((f) => f.configurationId === entry.configurationId)
  if (duplicate && duplicate.path !== entry.path) {
    console.warn(
      `[iii-ui] duplicate config-form override for '${entry.configurationId}' — ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return configFormsStore.add(entry)
}

/** Duplicate provider id: last registration wins in lookups. */
export function registerExtProviderConfigForm(
  entry: RegisteredProviderConfigForm,
): () => void {
  const duplicate = providerConfigFormsStore
    .get()
    .find((form) => form.providerId === entry.providerId)
  if (duplicate && duplicate.path !== entry.path) {
    console.warn(
      `[iii-ui] duplicate provider config-form override for '${entry.providerId}' — ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return providerConfigFormsStore.add(entry)
}

/** Duplicate chip id: last registration wins in `useExtSessionChips`. */
export function registerExtSessionChip(
  entry: RegisteredSessionChip,
): () => void {
  const duplicate = sessionChipsStore.get().find((c) => c.id === entry.id)
  if (duplicate && duplicate.path !== entry.path) {
    console.warn(
      `[iii-ui] duplicate session chip id '${entry.id}' — ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return sessionChipsStore.add(entry)
}

/** Duplicate summary id: last registration wins in chat footer lookups. */
export function registerExtSessionTurnSummary(
  entry: RegisteredSessionTurnSummary,
): () => void {
  const duplicate = sessionTurnSummariesStore
    .get()
    .find((summary) => summary.id === entry.id)
  if (duplicate && duplicate.path !== entry.path) {
    console.warn(
      `[iii-ui] duplicate session turn-summary id '${entry.id}' - ` +
        `'${entry.path}' overrides '${duplicate.path}'`,
    )
  }
  return sessionTurnSummariesStore.add(entry)
}

export function getExtPages(): readonly RegisteredPage[] {
  return pagesStore.get()
}

export function getExtTriggerActivityRenderers(): readonly RegisteredTriggerActivityRenderer[] {
  return triggerActivityRenderersStore.get()
}

/** Last registration wins for duplicate ids. */
export function getExtPage(id: string): RegisteredPage | undefined {
  const pages = pagesStore.get()
  for (let i = pages.length - 1; i >= 0; i--) {
    if (pages[i].id === id) return pages[i]
  }
  return undefined
}

/** The injected form override for one configuration id (last wins). */
export function getExtConfigForm(
  configurationId: string,
  fallbackConfigurationId?: string,
): RegisteredConfigForm | undefined {
  const forms = configFormsStore.get()
  for (const candidate of [configurationId, fallbackConfigurationId]) {
    if (!candidate) continue
    for (let i = forms.length - 1; i >= 0; i--) {
      if (forms[i].configurationId === candidate) return forms[i]
    }
  }
  return undefined
}

/** The injected form override for one model provider (last wins). */
export function getExtProviderConfigForm(
  providerId: string,
): RegisteredProviderConfigForm | undefined {
  const forms = providerConfigFormsStore.get()
  for (let i = forms.length - 1; i >= 0; i--) {
    if (forms[i].providerId === providerId) return forms[i]
  }
  return undefined
}

export function getUiAssetsStatus(): UiAssetsStatus {
  return uiAssetsStatusStore.get()
}

export function setUiAssetsStatus(status: UiAssetsStatus): void {
  uiAssetsStatusStore.set(status)
}

export function isExtConfigFormPending(
  status: UiAssetsStatus,
  form: RegisteredConfigForm | undefined,
): boolean {
  return status === 'loading' && form === undefined
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

export function useExtTriggerActivityRenderers(): readonly RegisteredTriggerActivityRenderer[] {
  return useSyncExternalStore(
    triggerActivityRenderersStore.subscribe,
    triggerActivityRenderersStore.get,
    () => EMPTY,
  )
}

/** All injected worker configuration forms, in registration order. */
export function useExtConfigForms(): readonly RegisteredConfigForm[] {
  return useSyncExternalStore(
    configFormsStore.subscribe,
    configFormsStore.get,
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
 * registration wins, matching the pages slot). Memoized on the store
 * snapshot so consumers get a stable array between registrations —
 * ChatView renders per streamed token, and its chip memo must hold.
 */
export function useExtSessionChips(): readonly RegisteredSessionChip[] {
  const chips = useSyncExternalStore(
    sessionChipsStore.subscribe,
    sessionChipsStore.get,
    () => EMPTY,
  )
  return useMemo(() => dedupeSessionChips(chips), [chips])
}

function dedupeSessionTurnSummaries(
  summaries: readonly RegisteredSessionTurnSummary[],
): readonly RegisteredSessionTurnSummary[] {
  const byId = new Map<string, RegisteredSessionTurnSummary>()
  for (const summary of summaries) byId.set(summary.id, summary)
  return [...byId.values()]
}

/** Footer turn summaries deduplicated by id; last registration wins. */
export function getExtSessionTurnSummaries(): readonly RegisteredSessionTurnSummary[] {
  return dedupeSessionTurnSummaries(sessionTurnSummariesStore.get())
}

export function useExtSessionTurnSummaries(): readonly RegisteredSessionTurnSummary[] {
  const summaries = useSyncExternalStore(
    sessionTurnSummariesStore.subscribe,
    sessionTurnSummariesStore.get,
    () => EMPTY,
  )
  return useMemo(() => dedupeSessionTurnSummaries(summaries), [summaries])
}

/** The injected form override for one configuration id (last wins). */
export function useExtConfigForm(
  configurationId: string,
  fallbackConfigurationId?: string,
): RegisteredConfigForm | undefined {
  const forms = useExtConfigForms()
  for (const candidate of [configurationId, fallbackConfigurationId]) {
    if (!candidate) continue
    for (let i = forms.length - 1; i >= 0; i--) {
      if (forms[i].configurationId === candidate) return forms[i]
    }
  }
  return undefined
}

/** The injected form override for one model provider (last wins). */
export function useExtProviderConfigForm(
  providerId: string,
): RegisteredProviderConfigForm | undefined {
  const forms = useSyncExternalStore(
    providerConfigFormsStore.subscribe,
    providerConfigFormsStore.get,
    () => EMPTY,
  )
  for (let i = forms.length - 1; i >= 0; i--) {
    if (forms[i].providerId === providerId) return forms[i]
  }
  return undefined
}

/**
 * Initial injected assets must settle before a consumer can distinguish
 * "there is no override" from "the override has not loaded yet".
 */
export function useUiAssetsStatus(): UiAssetsStatus {
  return useSyncExternalStore(
    uiAssetsStatusStore.subscribe,
    uiAssetsStatusStore.get,
    () => 'unavailable',
  )
}
