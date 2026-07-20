import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Typed wrappers over the optional `github` worker (the gh CLI as
 * `github::*` functions): the read-only list/search surface the Github page
 * renders. Every wrapper unwraps the worker's `{ value }` envelope and
 * parses rows with a tolerant zod schema (unknown fields ignored, invalid
 * rows dropped). Callers gate on presence (`use-github-status`).
 *
 * Mutating `github::*` functions are deliberately NOT wrapped here — writes
 * stay in agent flows where the approval gate reviews them.
 */

export const GITHUB_WORKER_NAME = 'github'

export const GITHUB_PR_LIST_FN = 'github::pr::list'
export const GITHUB_PR_CHECKS_FN = 'github::pr::checks'
export const GITHUB_ISSUE_LIST_FN = 'github::issue::list'
export const GITHUB_RUN_LIST_FN = 'github::run::list'
export const GITHUB_RELEASE_LIST_FN = 'github::release::list'

export const GITHUB_SEARCH_FNS = {
  repos: 'github::search::repos',
  issues: 'github::search::issues',
  prs: 'github::search::prs',
  code: 'github::search::code',
} as const
export type SearchKind = keyof typeof GITHUB_SEARCH_FNS

export const PR_STATE_FILTERS = ['open', 'merged', 'closed', 'all'] as const
export type PrStateFilter = (typeof PR_STATE_FILTERS)[number]

export const ISSUE_STATE_FILTERS = ['open', 'closed', 'all'] as const
export type IssueStateFilter = (typeof ISSUE_STATE_FILTERS)[number]

// ---- wire schemas (loose: gh adds fields freely; rows must not vanish
// because an optional field changed shape) ----

const authorSchema = z
  .object({ login: z.string().optional() })
  .nullable()
  .optional()

const labelSchema = z.object({
  name: z.string(),
  color: z.string().optional(),
})

const prSchema = z.object({
  number: z.number(),
  title: z.string(),
  state: z.string(),
  url: z.string(),
  author: authorSchema,
  headRefName: z.string().optional(),
  baseRefName: z.string().optional(),
  isDraft: z.boolean().optional(),
  labels: z.array(labelSchema).optional(),
  createdAt: z.string().optional(),
  updatedAt: z.string().optional(),
})
export type GithubPr = z.infer<typeof prSchema>

const prCheckSchema = z.object({
  bucket: z.string(),
  name: z.string(),
  state: z.string().optional(),
  link: z.string().nullable().optional(),
  workflow: z.string().nullable().optional(),
  description: z.string().nullable().optional(),
  startedAt: z.string().nullable().optional(),
  completedAt: z.string().nullable().optional(),
})
export type GithubPrCheck = z.infer<typeof prCheckSchema>

const issueSchema = z.object({
  number: z.number(),
  title: z.string(),
  state: z.string(),
  url: z.string(),
  author: authorSchema,
  labels: z.array(labelSchema).optional(),
  assignees: z.array(z.object({ login: z.string().optional() })).optional(),
  milestone: z.object({ title: z.string().optional() }).nullable().optional(),
  createdAt: z.string().optional(),
  updatedAt: z.string().optional(),
})
export type GithubIssue = z.infer<typeof issueSchema>

const runSchema = z.object({
  databaseId: z.number(),
  number: z.number().optional(),
  displayTitle: z.string().optional(),
  name: z.string().optional(),
  workflowName: z.string().optional(),
  headBranch: z.string().nullable().optional(),
  headSha: z.string().optional(),
  event: z.string().optional(),
  status: z.string().optional(),
  conclusion: z.string().nullable().optional(),
  attempt: z.number().optional(),
  createdAt: z.string().optional(),
  startedAt: z.string().optional(),
  updatedAt: z.string().optional(),
  url: z.string().optional(),
})
export type GithubRun = z.infer<typeof runSchema>

const releaseSchema = z.object({
  tagName: z.string(),
  name: z.string().nullable().optional(),
  isDraft: z.boolean().optional(),
  isLatest: z.boolean().optional(),
  isPrerelease: z.boolean().optional(),
  createdAt: z.string().optional(),
  publishedAt: z.string().nullable().optional(),
})
export type GithubRelease = z.infer<typeof releaseSchema>

const searchRepoSchema = z.object({
  fullName: z.string(),
  description: z.string().nullable().optional(),
  url: z.string(),
  visibility: z.string().optional(),
  isArchived: z.boolean().optional(),
  isFork: z.boolean().optional(),
  stargazersCount: z.number().optional(),
  forksCount: z.number().optional(),
  language: z.string().nullable().optional(),
  updatedAt: z.string().optional(),
})
export type GithubSearchRepo = z.infer<typeof searchRepoSchema>

const searchRepositoryRefSchema = z
  .object({
    nameWithOwner: z.string().optional(),
    name: z.string().optional(),
  })
  .optional()

const searchIssueSchema = z.object({
  number: z.number(),
  title: z.string(),
  state: z.string(),
  url: z.string(),
  repository: searchRepositoryRefSchema,
  author: authorSchema,
  labels: z.array(labelSchema).optional(),
  commentsCount: z.number().optional(),
  createdAt: z.string().optional(),
  updatedAt: z.string().optional(),
  isDraft: z.boolean().optional(),
})
export type GithubSearchIssue = z.infer<typeof searchIssueSchema>

const searchCodeSchema = z.object({
  path: z.string(),
  repository: searchRepositoryRefSchema,
  sha: z.string().optional(),
  url: z.string().optional(),
  textMatches: z.array(z.unknown()).optional(),
})
export type GithubSearchCode = z.infer<typeof searchCodeSchema>

// ---- parsing (exported for tests; wrappers below are thin) ----

const valueEnvelopeSchema = z.object({ value: z.unknown() })

/** Unwrap the worker's `ValueResponse { value }` envelope. */
export function unwrapValue(res: unknown): unknown {
  const parsed = valueEnvelopeSchema.safeParse(res)
  return parsed.success ? parsed.data.value : null
}

function parseArray<T>(value: unknown, schema: z.ZodType<T>): T[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((item) => {
    const parsed = schema.safeParse(item)
    return parsed.success ? [parsed.data] : []
  })
}

export function parsePrs(value: unknown): GithubPr[] {
  return parseArray(value, prSchema)
}
export function parsePrChecks(value: unknown): GithubPrCheck[] {
  return parseArray(value, prCheckSchema)
}
export function parseIssues(value: unknown): GithubIssue[] {
  return parseArray(value, issueSchema)
}
export function parseRuns(value: unknown): GithubRun[] {
  return parseArray(value, runSchema)
}
export function parseReleases(value: unknown): GithubRelease[] {
  return parseArray(value, releaseSchema)
}

export type GithubSearchItems =
  | { kind: 'repos'; items: GithubSearchRepo[] }
  | { kind: 'issues'; items: GithubSearchIssue[] }
  | { kind: 'prs'; items: GithubSearchIssue[] }
  | { kind: 'code'; items: GithubSearchCode[] }

export function parseSearchItems(
  kind: SearchKind,
  value: unknown,
): GithubSearchItems {
  switch (kind) {
    case 'repos':
      return { kind, items: parseArray(value, searchRepoSchema) }
    case 'issues':
      return { kind, items: parseArray(value, searchIssueSchema) }
    case 'prs':
      return { kind, items: parseArray(value, searchIssueSchema) }
    case 'code':
      return { kind, items: parseArray(value, searchCodeSchema) }
  }
}

// ---- RPC wrappers ----

const LIST_LIMIT = 30

async function triggerValue(
  fnId: string,
  payload: Record<string, unknown>,
): Promise<unknown> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(fnId, payload)
  return unwrapValue(res)
}

export async function listPrs(
  repo: string,
  state: PrStateFilter,
): Promise<GithubPr[]> {
  return parsePrs(
    await triggerValue(GITHUB_PR_LIST_FN, { repo, state, limit: LIST_LIMIT }),
  )
}

export async function listPrChecks(
  repo: string,
  number: number,
): Promise<GithubPrCheck[]> {
  return parsePrChecks(
    await triggerValue(GITHUB_PR_CHECKS_FN, { repo, number }),
  )
}

export async function listIssues(
  repo: string,
  state: IssueStateFilter,
): Promise<GithubIssue[]> {
  return parseIssues(
    await triggerValue(GITHUB_ISSUE_LIST_FN, {
      repo,
      state,
      limit: LIST_LIMIT,
    }),
  )
}

export async function listRuns(repo: string): Promise<GithubRun[]> {
  return parseRuns(
    await triggerValue(GITHUB_RUN_LIST_FN, { repo, limit: LIST_LIMIT }),
  )
}

export async function listReleases(repo: string): Promise<GithubRelease[]> {
  return parseReleases(
    await triggerValue(GITHUB_RELEASE_LIST_FN, { repo, limit: LIST_LIMIT }),
  )
}

export async function searchGithub(
  kind: SearchKind,
  query: string,
): Promise<GithubSearchItems> {
  return parseSearchItems(
    kind,
    await triggerValue(GITHUB_SEARCH_FNS[kind], { query, limit: LIST_LIMIT }),
  )
}

// ---- small view helpers ----

/** The releases list payload carries no url; build the canonical one. */
export function releaseUrl(repo: string, tagName: string): string {
  return `https://github.com/${repo}/releases/tag/${encodeURIComponent(tagName)}`
}

/** Compact relative time for gh's ISO timestamps ("3h ago"). */
export function timeAgoIso(
  iso: string | null | undefined,
  now = Date.now(),
): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ''
  const s = Math.max(0, Math.floor((now - t) / 1000))
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h ago`
  const d = Math.floor(h / 24)
  if (d < 30) return `${d}d ago`
  const mo = Math.floor(d / 30)
  if (mo < 12) return `${mo}mo ago`
  return `${Math.floor(mo / 12)}y ago`
}

// ---- repo persistence (UI affordance only) ----

const GITHUB_REPO_KEY = 'iii-github-repo'

export function loadGithubRepo(): string {
  try {
    return localStorage.getItem(GITHUB_REPO_KEY) ?? ''
  } catch {
    return ''
  }
}

export function saveGithubRepo(repo: string): void {
  try {
    if (repo) localStorage.setItem(GITHUB_REPO_KEY, repo)
    else localStorage.removeItem(GITHUB_REPO_KEY)
  } catch {
    // storage unavailable; the field just won't persist
  }
}
