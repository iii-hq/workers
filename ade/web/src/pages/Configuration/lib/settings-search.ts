/**
 * Settings search: one index over every settings page and the fields inside
 * it, matched the way people type into a settings box.
 *
 * - Every query word must match (AND), and it must match the START of a word
 *   ("port" finds "HTTP port", never "support" or "report"). A word may run
 *   across the label's own word breaks, so "github" finds "GitHub", "vscode"
 *   finds "VS Code" and "pubsub" finds "Pub/Sub".
 * - A page ranks by where the words landed: its name, then a field label,
 *   then a field reached through its section, then descriptions.
 * - Namespaced runtime ids (`hangzhou-ade`) are never matched on their shared
 *   prefix, which would otherwise make one typed letter match every worker.
 *
 * The index is built once per catalog change; a keystroke only walks
 * pre-split word arrays, so search stays instant at any catalog size.
 */

import type { JsonSchema } from '../tabs/WorkersTab/api'
import { resolveSchema, withoutNull } from './schema/ref-resolver'

export interface SettingsSearchField {
  /** Unique within its page. */
  key: string
  label: string
  /** Section labels leading to the field, outermost first. */
  trail: string[]
  description?: string
  /** Extra words that should find the field: enum values, synonyms. */
  keywords?: string[]
  /** Worker field path, for the deep link. */
  path?: string[]
  /** DOM id of the General row, for the scroll target. */
  anchor?: string
  /** DOM id to scroll to when `anchor` is not mounted (a hidden row). */
  fallbackAnchor?: string
}

export interface SettingsSearchDoc {
  /** `general`, or the configuration id. */
  id: string
  name: string
  /** Other names the page answers to (its form family, its id suffix). */
  aliases: string[]
  description: string
  fields: SettingsSearchField[]
}

export interface SettingsSearchFieldHit {
  field: SettingsSearchField
  score: number
}

export interface SettingsSearchResult {
  doc: SettingsSearchDoc
  score: number
  /** Fields that matched on their own words, best first. */
  fields: SettingsSearchFieldHit[]
}

interface IndexedField {
  field: SettingsSearchField
  label: string[]
  trail: string[]
  extra: string[]
}

interface IndexedDoc {
  doc: SettingsSearchDoc
  name: string[]
  nameCompact: string
  aliases: string[]
  description: string[]
  fields: IndexedField[]
}

export interface SettingsSearchIndex {
  docs: IndexedDoc[]
  /** Words a correction may suggest: names, labels, sections, keywords. */
  vocabulary: string[]
}

/* ------------------------------------------------------------------ */
/*  Text                                                               */
/* ------------------------------------------------------------------ */

/** Lowercase, accent-free, punctuation as spaces. */
export function normalizeSearchText(text: string): string {
  return text
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
}

function words(...texts: (string | undefined)[]): string[] {
  const out: string[] = []
  for (const text of texts) {
    if (!text) continue
    for (const word of normalizeSearchText(text).split(' ')) {
      if (word) out.push(word)
    }
  }
  return out
}

/** `httpPort` / `http_port` → `http port`, for schema keys. */
function splitKey(key: string): string {
  return key
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
}

/** The query as terms. (A plural term also matches its singular; see `termIn`.) */
export function searchTerms(query: string): string[] {
  return words(query)
}

/** Does `term` start at `words[i]`, possibly running across word breaks? */
function termAt(term: string, list: readonly string[], i: number): boolean {
  let acc = ''
  for (let j = i; j < list.length && j < i + 4; j++) {
    acc += list[j]
    if (acc.startsWith(term)) return true
    if (!term.startsWith(acc)) return false
  }
  return false
}

function termIn(term: string, ...lists: (readonly string[])[]): boolean {
  const singular =
    term.length > 3 && term.endsWith('s') ? term.slice(0, -1) : null
  for (const list of lists) {
    for (let i = 0; i < list.length; i++) {
      if (termAt(term, list, i)) return true
      if (singular && termAt(singular, list, i)) return true
    }
  }
  return false
}

function allIn(terms: readonly string[], ...lists: (readonly string[])[]) {
  return terms.every((term) => termIn(term, ...lists))
}

function anyIn(terms: readonly string[], ...lists: (readonly string[])[]) {
  return terms.some((term) => termIn(term, ...lists))
}

/**
 * Prose matches whole words of three letters or more (or their plural or
 * singular): a prefix search over a paragraph finds "themselves" for
 * "theme" and "has" for "ha".
 */
function wordIn(term: string, list: readonly string[]): boolean {
  // One or two letters are a name being typed, not a word to look for.
  if (term.length < 3) return false
  const singular =
    term.length > 3 && term.endsWith('s') ? term.slice(0, -1) : null
  return list.some(
    (word) => word === term || word === `${term}s` || word === singular,
  )
}

/** Every term either starts a word of `labels` or is a word of `prose`. */
function allInLabelsOrProse(
  terms: readonly string[],
  labels: (readonly string[])[],
  prose: readonly string[],
): boolean {
  return terms.every((term) => termIn(term, ...labels) || wordIn(term, prose))
}

/**
 * Character ranges of `text` to highlight for `query`: each term where it
 * starts a word. Overlaps merge; ranges come back sorted.
 */
export function highlightRanges(
  text: string,
  query: string,
): [number, number][] {
  const terms = searchTerms(query)
  if (terms.length === 0) return []
  const lower = text.toLowerCase()
  const ranges: [number, number][] = []
  for (const term of terms) {
    let from = 0
    while (from <= lower.length - term.length) {
      const at = lower.indexOf(term, from)
      if (at < 0) break
      const prev = at > 0 ? text[at - 1] : ''
      const boundary =
        at === 0 ||
        !/[a-z0-9]/i.test(prev) ||
        (/[a-z]/.test(prev) && /[A-Z]/.test(text[at]))
      if (boundary) ranges.push([at, at + term.length])
      from = at + 1
    }
  }
  ranges.sort((a, b) => a[0] - b[0])
  const merged: [number, number][] = []
  for (const range of ranges) {
    const last = merged[merged.length - 1]
    if (last && range[0] <= last[1]) last[1] = Math.max(last[1], range[1])
    else merged.push([range[0], range[1]])
  }
  return merged
}

/* ------------------------------------------------------------------ */
/*  Index + query                                                      */
/* ------------------------------------------------------------------ */

export function buildSettingsSearchIndex(
  docs: readonly SettingsSearchDoc[],
): SettingsSearchIndex {
  const indexed = docs.map(
    (doc): IndexedDoc => ({
      doc,
      name: words(doc.name),
      nameCompact: normalizeSearchText(doc.name).replace(/ /g, ''),
      aliases: words(...doc.aliases.map(splitKey)),
      description: words(doc.description),
      fields: doc.fields.map((field) => ({
        field,
        label: words(
          field.label,
          ...(field.path?.slice(-1).map(splitKey) ?? []),
        ),
        trail: words(...field.trail),
        extra: words(field.description, ...(field.keywords ?? [])),
      })),
    }),
  )
  const vocabulary = new Set<string>()
  for (const doc of indexed) {
    for (const word of [...doc.name, ...doc.aliases]) vocabulary.add(word)
    for (const field of doc.fields) {
      for (const word of [...field.label, ...field.trail]) vocabulary.add(word)
      for (const word of words(...(field.field.keywords ?? []))) {
        vocabulary.add(word)
      }
    }
  }
  return {
    docs: indexed,
    vocabulary: [...vocabulary].filter((word) => word.length >= 3),
  }
}

const SCORE = {
  nameExact: 1000,
  namePrefix: 900,
  nameWords: 800,
  alias: 700,
  fieldLabelPrefix: 650,
  fieldLabel: 600,
  fieldInContext: 500,
  fieldDescription: 350,
  description: 300,
} as const

function scoreField(
  terms: readonly string[],
  compactQuery: string,
  entry: IndexedField,
  doc: IndexedDoc,
): number {
  const { label, trail, extra } = entry
  if (allIn(terms, label)) {
    return label.join('').startsWith(compactQuery)
      ? SCORE.fieldLabelPrefix
      : SCORE.fieldLabel
  }
  // "appearance theme", "github token": the rest of the words may land on
  // the section or the page, but the field must carry at least one.
  if (
    anyIn(terms, label) &&
    allIn(terms, label, trail, doc.name, doc.aliases)
  ) {
    return SCORE.fieldInContext
  }
  if (
    (anyIn(terms, label) || terms.some((term) => wordIn(term, extra))) &&
    allInLabelsOrProse(terms, [label, trail, doc.name, doc.aliases], extra)
  ) {
    return SCORE.fieldDescription
  }
  return 0
}

/**
 * Every page that matches `query`, best first, each with its matching
 * fields. An empty query returns nothing: the caller shows the full catalog.
 */
export function searchSettings(
  index: SettingsSearchIndex,
  query: string,
): SettingsSearchResult[] {
  const terms = searchTerms(query)
  if (terms.length === 0) return []
  const compactQuery = terms.join('')
  const results: SettingsSearchResult[] = []

  for (const doc of index.docs) {
    let score = 0
    if (doc.nameCompact === compactQuery) score = SCORE.nameExact
    else if (doc.nameCompact.startsWith(compactQuery)) score = SCORE.namePrefix
    else if (allIn(terms, doc.name)) score = SCORE.nameWords
    else if (allIn(terms, doc.name, doc.aliases)) score = SCORE.alias

    const fields: SettingsSearchFieldHit[] = []
    for (const entry of doc.fields) {
      const fieldScore = scoreField(terms, compactQuery, entry, doc)
      if (fieldScore > 0) fields.push({ field: entry.field, score: fieldScore })
    }
    fields.sort((a, b) => b.score - a.score)
    // Fields found only through their description are listed when nothing
    // better is: not under a page that matched by name, and not beside a
    // field whose own label matched.
    const strongest = Math.max(score, fields[0]?.score ?? 0)
    const shown =
      strongest > SCORE.fieldDescription
        ? fields.filter((hit) => hit.score > SCORE.fieldDescription)
        : fields
    score = strongest

    if (
      score === 0 &&
      allInLabelsOrProse(terms, [doc.name, doc.aliases], doc.description)
    ) {
      score = SCORE.description
    }
    if (score > 0) results.push({ doc: doc.doc, score, fields: shown })
  }

  return results.sort(
    (a, b) => b.score - a.score || a.doc.name.localeCompare(b.doc.name),
  )
}

/** Levenshtein distance, giving up past `max`. */
function editDistance(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i++) {
    const next = [i]
    let rowMin = i
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1
      next[j] = Math.min(prev[j] + 1, next[j - 1] + 1, prev[j - 1] + cost)
      rowMin = Math.min(rowMin, next[j])
    }
    if (rowMin > max) return max + 1
    prev = next
  }
  return prev[b.length]
}

/**
 * For a query with no results, the nearest spelling that has some
 * ("githb" → "github"). Each unmatched term is swapped for the closest
 * catalog word within one edit (two for longer words); null when no
 * correction finds anything. Results themselves stay exact: a typo is
 * offered back as a suggestion, never silently matched.
 */
export function suggestSettingsQuery(
  index: SettingsSearchIndex,
  query: string,
): string | null {
  const terms = searchTerms(query)
  if (terms.length === 0) return null
  let changed = false
  const corrected = terms.map((term) => {
    if (term.length < 3 || index.vocabulary.some((w) => termAt(term, [w], 0))) {
      return term
    }
    const max = term.length >= 7 ? 2 : 1
    let best: string | null = null
    let bestDistance = max + 1
    for (const word of index.vocabulary) {
      const candidate =
        word.length > term.length + max ? word.slice(0, term.length) : word
      const distance = editDistance(term, candidate, max)
      if (
        distance < bestDistance ||
        (distance === bestDistance &&
          best !== null &&
          word.length < best.length)
      ) {
        best = word
        bestDistance = distance
      }
    }
    if (best === null) return term
    changed = true
    return best
  })
  if (!changed) return null
  const suggestion = corrected.join(' ')
  return searchSettings(index, suggestion).length > 0 ? suggestion : null
}

/* ------------------------------------------------------------------ */
/*  Schema → fields                                                    */
/* ------------------------------------------------------------------ */

const ACRONYMS = new Set([
  'ade',
  'api',
  'cors',
  'cpu',
  'css',
  'db',
  'fs',
  'gpu',
  'html',
  'http',
  'https',
  'id',
  'ip',
  'json',
  'llm',
  'mcp',
  'otel',
  'sql',
  'ssh',
  'stt',
  'tls',
  'tts',
  'ttl',
  'ui',
  'uri',
  'url',
])

/** `http_port` → `HTTP port`, `injectableUi` → `Injectable UI`. */
export function humanizeKey(key: string): string {
  const parts = splitKey(key)
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((part) => part.toLowerCase())
  return parts
    .map((part, i) => {
      if (ACRONYMS.has(part)) return part.toUpperCase()
      return i === 0 ? part.charAt(0).toUpperCase() + part.slice(1) : part
    })
    .join(' ')
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

const MAX_DEPTH = 4
const MAX_FIELDS = 400

/** One schema node with `$ref`, `allOf` wrappers and `null` unions resolved. */
function settle(node: unknown, root: JsonSchema): JsonSchema {
  if (!isRecord(node)) return {}
  return withoutNull(resolveSchema(node as JsonSchema, { root }))
}

/** The object shapes a node can take: itself, or each `oneOf`/`anyOf` arm. */
function objectShapes(schema: JsonSchema, root: JsonSchema): JsonSchema[] {
  if (isRecord(schema.properties)) return [schema]
  const arms = [schema.oneOf, schema.anyOf].find(Array.isArray) as
    | unknown[]
    | undefined
  if (!arms) return []
  return arms
    .map((arm) => settle(arm, root))
    .filter((arm) => isRecord(arm.properties))
}

function enumWords(schema: JsonSchema, root: JsonSchema): string[] {
  const out: string[] = []
  const push = (value: unknown) => {
    if (typeof value === 'string') out.push(value)
  }
  if (Array.isArray(schema.enum)) schema.enum.forEach(push)
  push(schema.const)
  for (const arms of [schema.oneOf, schema.anyOf]) {
    if (!Array.isArray(arms)) continue
    for (const arm of arms) {
      const settled = settle(arm, root)
      if (Array.isArray(settled.enum)) settled.enum.forEach(push)
      push(settled.const)
    }
  }
  return out
}

/**
 * The searchable fields of a worker's JSON schema: every named property down
 * to a few levels, labelled by its `title` or its humanized key, with its
 * section trail, description and enum values. Array items and free-form maps
 * contribute their property names as keywords of the list itself (there is
 * no single field to link to inside them).
 */
export function schemaSearchFields(
  schema: JsonSchema | null | undefined,
): SettingsSearchField[] {
  if (!isRecord(schema)) return []
  const root = schema
  const fields: SettingsSearchField[] = []
  const seen = new Set<string>()

  const walk = (
    node: JsonSchema,
    path: string[],
    trail: string[],
    depth: number,
  ) => {
    for (const shape of objectShapes(node, root)) {
      const properties = shape.properties as Record<string, unknown>
      for (const [key, raw] of Object.entries(properties)) {
        if (fields.length >= MAX_FIELDS) return
        const child = settle(raw, root)
        const childPath = [...path, key]
        const id = childPath.join('/')
        const label =
          typeof child.title === 'string' && child.title.trim()
            ? child.title.trim()
            : humanizeKey(key)
        const description =
          typeof child.description === 'string'
            ? child.description
            : isRecord(raw) && typeof raw.description === 'string'
              ? raw.description
              : undefined
        const keywords = enumWords(child, root)
        const items = settle(child.items, root)
        for (const itemShape of objectShapes(items, root)) {
          for (const itemKey of Object.keys(
            itemShape.properties as Record<string, unknown>,
          )) {
            keywords.push(splitKey(itemKey))
          }
        }
        if (!seen.has(id)) {
          seen.add(id)
          fields.push({
            key: id,
            label,
            trail,
            description,
            keywords,
            path: childPath,
          })
        }
        if (depth < MAX_DEPTH)
          walk(child, childPath, [...trail, label], depth + 1)
      }
    }
  }

  walk(root, [], [], 1)
  return fields
}

/**
 * Leading `name-` segments shared by two or more ids (a compose namespace
 * such as `hangzhou-`). Stripped before ids are indexed: matched, they would
 * make one typed letter find every worker in the namespace.
 */
export function sharedIdPrefixes(ids: readonly string[]): Set<string> {
  const counts = new Map<string, number>()
  for (const id of ids) {
    const cut = id.indexOf('-')
    if (cut <= 0) continue
    const prefix = id.slice(0, cut + 1)
    counts.set(prefix, (counts.get(prefix) ?? 0) + 1)
  }
  return new Set(
    [...counts].filter(([, count]) => count >= 2).map(([prefix]) => prefix),
  )
}

/** `id` without a shared namespace prefix. */
export function stripIdPrefix(
  id: string,
  prefixes: ReadonlySet<string>,
): string {
  const cut = id.indexOf('-')
  if (cut <= 0) return id
  return prefixes.has(id.slice(0, cut + 1)) ? id.slice(cut + 1) : id
}
