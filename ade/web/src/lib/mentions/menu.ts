/**
 * The composer's `@` menu as data: which rows show, in which groups, for a
 * query. Pure, so the grouping and the scoped `@<provider>:` mode are
 * testable without an editor.
 *
 * - **Global** (`@text`): the providers whose name matches ("sources" —
 *   picking one drills into it), then — from two characters on — one group
 *   per provider with its best few items and a "more <label>" row that
 *   drills in carrying the text, then the functions & files list as it
 *   always was.
 * - **Scoped** (`@kanban:text`): that provider's items only.
 */

import type { MentionCandidate } from '@/lib/mention-search'
import type { ProviderResults } from './search'
import type { MentionItem, MentionProvider } from './types'

/** Items per provider group in the global menu (searched with one extra). */
export const GLOBAL_PROVIDER_ROWS = 3
export const GLOBAL_PROVIDER_LIMIT = GLOBAL_PROVIDER_ROWS + 1
/** Items in a scoped (`@provider:`) menu. */
export const SCOPED_PROVIDER_ROWS = 20
/** Characters before the global menu asks providers at all. */
export const PROVIDER_MIN_QUERY = 2

export type MentionMenuRow =
  /** A function or a file (the catalog list). */
  | { kind: 'candidate'; candidate: MentionCandidate }
  /** Reveal the catalog list's next page. */
  | { kind: 'more'; remaining: number }
  /** A provider: picking it drills into `@<name>:`. */
  | { kind: 'provider'; provider: MentionProvider }
  /** One provider item: picking it inserts the mention. */
  | { kind: 'entity'; provider: MentionProvider; item: MentionItem }
  /** Drill into `@<name>:<query>` for more of that provider's items. */
  | { kind: 'provider-more'; provider: MentionProvider; query: string }

export interface MentionMenuSection {
  key: string
  label: string
  provider?: MentionProvider
}

export interface MentionMenuEntry {
  key: string
  section: MentionMenuSection
  row: MentionMenuRow
}

export interface ScopedQuery {
  provider: MentionProvider
  query: string
}

const SCOPED_RE = /^([a-z][a-z0-9-]{0,39}):(?!:)([\s\S]*)$/

/**
 * `kanban:fix login` → the kanban provider and `fix login`, when a provider
 * by that name exists. A double colon (`kanban::ticket::get`) is a
 * function id, never a scope.
 */
export function parseScopedQuery(
  matching: string,
  byName: ReadonlyMap<string, MentionProvider>,
): ScopedQuery | null {
  const m = SCOPED_RE.exec(matching)
  if (!m) return null
  const provider = byName.get(m[1])
  return provider ? { provider, query: m[2] } : null
}

/** The text that scopes the menu to a provider: `@kanban:` + query. */
export function scopedText(provider: MentionProvider, query = ''): string {
  return `@${provider.name}:${query}`
}

/** Providers whose name (or label) starts with the query; all for none. */
export function matchingProviders(
  query: string,
  providers: readonly MentionProvider[],
): MentionProvider[] {
  const q = query.trim().toLowerCase()
  if (!q) return [...providers]
  return providers.filter(
    (provider) =>
      provider.name.startsWith(q) || provider.label.toLowerCase().startsWith(q),
  )
}

const SOURCES: MentionMenuSection = { key: 'sources', label: 'mention' }
const CATALOG: MentionMenuSection = {
  key: 'catalog',
  label: 'functions & files',
}

function providerSection(provider: MentionProvider): MentionMenuSection {
  return { key: `provider:${provider.name}`, label: provider.label, provider }
}

function entityEntry(
  provider: MentionProvider,
  item: MentionItem,
): MentionMenuEntry {
  return {
    key: `entity:${provider.name}:${item.id}`,
    section: providerSection(provider),
    row: { kind: 'entity', provider, item },
  }
}

export interface GlobalMenuInput {
  query: string
  providers: readonly MentionProvider[]
  /** The catalog list's visible page (already ranked and paged). */
  catalog: readonly MentionCandidate[]
  /** Catalog rows past the visible page. */
  catalogRemaining: number
  catalogKey: (candidate: MentionCandidate) => string
  results: ReadonlyMap<string, ProviderResults>
}

export function buildGlobalMenu({
  query,
  providers,
  catalog,
  catalogRemaining,
  catalogKey,
  results,
}: GlobalMenuInput): MentionMenuEntry[] {
  const entries: MentionMenuEntry[] = []
  for (const provider of matchingProviders(query, providers)) {
    entries.push({
      key: `provider:${provider.name}`,
      section: SOURCES,
      row: { kind: 'provider', provider },
    })
  }
  // Worker groups come before functions & files: ten file hits would
  // otherwise push them out of the menu's view.
  if (query.trim().length >= PROVIDER_MIN_QUERY) {
    for (const provider of providers) {
      const items = results.get(provider.name)?.items ?? []
      if (items.length === 0) continue
      for (const item of items.slice(0, GLOBAL_PROVIDER_ROWS)) {
        entries.push(entityEntry(provider, item))
      }
      // The search asks for one row more than it shows, to know.
      if (items.length > GLOBAL_PROVIDER_ROWS) {
        entries.push({
          key: `provider-more:${provider.name}`,
          section: providerSection(provider),
          row: { kind: 'provider-more', provider, query: query.trim() },
        })
      }
    }
  }
  for (const candidate of catalog) {
    entries.push({
      key: catalogKey(candidate),
      section: CATALOG,
      row: { kind: 'candidate', candidate },
    })
  }
  if (catalogRemaining > 0) {
    entries.push({
      key: 'more',
      section: CATALOG,
      row: { kind: 'more', remaining: catalogRemaining },
    })
  }
  return entries
}

export function buildScopedMenu(
  scoped: ScopedQuery,
  results: ReadonlyMap<string, ProviderResults>,
): MentionMenuEntry[] {
  const items = results.get(scoped.provider.name)?.items ?? []
  return items
    .slice(0, SCOPED_PROVIDER_ROWS)
    .map((item) => entityEntry(scoped.provider, item))
}

/**
 * Whether the list draws group headers: when it spans more than one group,
 * or its one group is a worker's (so a lone ticket list still says
 * "Tickets"). A plain functions & files list stays as it always looked.
 */
export function showsSectionHeaders(
  entries: readonly MentionMenuEntry[],
): boolean {
  const first = entries[0]?.section
  if (!first) return false
  if (first.provider) return true
  return entries.some((entry) => entry.section.key !== first.key)
}
