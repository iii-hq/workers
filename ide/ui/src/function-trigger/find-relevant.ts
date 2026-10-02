import type { PanelOpenRequest } from '@iii-dev/console-ui'
import { z } from 'zod'

export const FIND_RELEVANT_ID = 'coder::find-relevant'

/* The coder::find-relevant wire shapes (ide/src/code/find_relevant/mod.rs).
   Lenient like every card schema: the card reads what it shows. */

const requestSchema = z.object({
  query: z.string(),
  path: z.string().optional(),
})

const lineRange = { line_from: z.number(), line_to: z.number() }

const fileSchema = z.object({
  path: z.string(),
  score: z.number(),
  priority: z.number().nullish(),
  roles: z.array(z.string()).default([]),
  excerpts: z.array(z.object({ ...lineRange, text: z.string(), partial: z.unknown().optional() })).default([]),
  leads: z.array(z.object({ ...lineRange, name: z.string().optional(), score: z.number().optional() })).default([]),
  source_omitted: z.boolean().default(false),
})

const responseSchema = z.object({
  status: z.enum(['complete', 'incomplete', 'unavailable']),
  reason: z.string().nullish(),
  files: z.array(fileSchema),
  issues: z.record(z.string(), z.number()).default({}),
  stats: z
    .object({
      judge_calls: z.number().default(0),
      elapsed_ms: z.number().default(0),
      cache_hits: z.number().default(0),
    })
    .default({ judge_calls: 0, elapsed_ms: 0, cache_hits: 0 }),
})

export type RelevantStatus = z.infer<typeof responseSchema>['status']

export interface RelevantExcerpt {
  lineFrom: number
  lineTo: number
  text: string
}

export interface RelevantLead {
  name: string
  lineFrom: number
  lineTo: number
  score: number
}

export interface RelevantRow {
  /** Absolute path, as the worker returned it; what the IDE opens. */
  path: string
  /** Folder part shown faint, relative to the rows' shared folder. */
  dir: string
  name: string
  /** `priority ?? score`: the value the worker ranked by, 0–1. */
  rank: number
  roles: string[]
  excerpts: RelevantExcerpt[]
  leads: RelevantLead[]
  sourceOmitted: boolean
}

export interface RelevantSummary {
  query: string
  /** The folder the ask walked, as the caller wrote it (`.` omitted). */
  scope: string | null
  /** Null while the call is in flight or only its request is known. */
  status: RelevantStatus | null
  reason: string | null
  rows: RelevantRow[]
  issues: [string, number][]
  judgeCalls: number
  elapsedMs: number
  cacheHits: number
}

export function isFindRelevantResponse(output: unknown): boolean {
  return responseSchema.safeParse(output).success
}

/** The request alone is enough for the in-flight card; the response, when
    it parses, fills the ranked rows. Null means "not this card". */
export function summarizeFindRelevant(input: unknown, output: unknown): RelevantSummary | null {
  const request = requestSchema.safeParse(input)
  if (!request.success) return null
  const response = output === undefined ? null : responseSchema.safeParse(output)
  if (response && !response.success) return null
  const data = response?.data
  const paths = data?.files.map((file) => file.path) ?? []
  const base = sharedFolder(paths)
  return {
    query: request.data.query,
    scope: request.data.path && request.data.path !== '.' ? request.data.path : null,
    status: data?.status ?? null,
    reason: data?.reason ?? null,
    rows: (data?.files ?? []).map((file) => {
      const rel = base && file.path.startsWith(base) ? file.path.slice(base.length) : file.path
      const cut = rel.lastIndexOf('/')
      return {
        path: file.path,
        dir: cut >= 0 ? rel.slice(0, cut + 1) : '',
        name: cut >= 0 ? rel.slice(cut + 1) : rel,
        rank: clamp01(file.priority ?? file.score),
        roles: file.roles,
        excerpts: file.excerpts.map((e) => ({ lineFrom: e.line_from, lineTo: e.line_to, text: e.text })),
        leads: file.leads.map((l) => ({
          name: l.name ?? '',
          lineFrom: l.line_from,
          lineTo: l.line_to,
          score: l.score ?? 0,
        })),
        sourceOmitted: file.source_omitted,
      }
    }),
    issues: Object.entries(data?.issues ?? {}),
    judgeCalls: data?.stats.judge_calls ?? 0,
    elapsedMs: data?.stats.elapsed_ms ?? 0,
    cacheHits: data?.stats.cache_hits ?? 0,
  }
}

/** Open the file in the IDE at a range (the page's `file` panel context). */
export function openFileRequest(path: string, lineFrom?: number, lineTo?: number): PanelOpenRequest {
  return {
    pageId: 'ide',
    context: {
      type: 'file',
      path,
      ...(lineFrom ? { line: lineFrom } : {}),
      ...(lineFrom && lineTo && lineTo >= lineFrom ? { endLine: lineTo } : {}),
    },
  }
}

/* Units the parsers could not name fall back to their syntax kind; as a
   lead they say where, not what. */
const UNNAMED_UNITS = new Set([
  'source',
  'comment',
  'use_declaration',
  'extern_crate_declaration',
  'attribute_item',
  'inner_attribute_item',
  'import_statement',
  'import_from_statement',
  'future_import_statement',
  'import_declaration',
  'package_clause',
  'expression_statement',
  'export_statement',
])

/** The leads worth a chip: named units, best judged first. */
export function namedLeads(row: RelevantRow): RelevantLead[] {
  return row.leads
    .filter((lead) => lead.name && !UNNAMED_UNITS.has(lead.name.slice(lead.name.lastIndexOf('.') + 1)))
    .sort((a, b) => b.score - a.score || a.lineFrom - b.lineFrom)
}

/** Where a row click lands: its first excerpt, else its first lead. */
export function firstLocation(row: RelevantRow): { lineFrom: number; lineTo: number } | null {
  const first = row.excerpts[0] ?? row.leads[0]
  return first ? { lineFrom: first.lineFrom, lineTo: first.lineTo } : null
}

/** The folder every path shares, with its trailing slash; `''` when the
    paths share nothing but the filesystem root. */
export function sharedFolder(paths: readonly string[]): string {
  if (paths.length === 0) return ''
  let prefix = paths[0].slice(0, paths[0].lastIndexOf('/') + 1)
  for (const path of paths.slice(1)) {
    while (prefix && !path.startsWith(prefix)) {
      prefix = prefix.slice(0, prefix.slice(0, -1).lastIndexOf('/') + 1)
    }
  }
  return prefix === '/' ? '' : prefix
}

/** An excerpt cut to its first `limit` lines, numbered from its start. */
export function previewLines(excerpt: RelevantExcerpt, limit: number): { number: number; text: string }[] {
  const lines = excerpt.text.replace(/\n$/, '').split('\n')
  return lines.slice(0, limit).map((text, index) => ({ number: excerpt.lineFrom + index, text }))
}

export function formatElapsed(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`
  return ms < 10_000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms / 1000)} s`
}

function clamp01(value: number): number {
  return Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0
}
