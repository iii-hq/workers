import type { GhRunner, PullRequestRef } from '@/lib/chat-aside'
import type { Conversation } from '@/types/chat'

/**
 * Auto-fix without the `github` worker: while the chat is open, the ADE reads
 * its pull requests with `gh` and hands the agent what changed since it last
 * reported — a check that started failing, a new review, a merge conflict.
 * What was last reported lives in the session's metadata, so every tab and
 * pane sees the same baseline and a chat opened later catches up.
 */

/** Session metadata key: the PRs auto-fix watches, by `repo#number`. */
export const AUTO_FIX_KEY = 'context_autofix'

/** What auto-fix last saw on a PR. */
export interface AutoFixSeen {
  url: string
  /** Checks failing at the last read. */
  failing: string[]
  /** Reviews by others asking for something, at the last read. */
  reviews: number
  conflict: boolean
  /** How many times it woke the agent: it stops at the cap. */
  reports: number
}

/** ponytail: a fixed cap, so a check the agent cannot fix stops waking it. */
export const AUTO_FIX_REPORT_CAP = 10

export function autoFixState(
  conversation: Pick<Conversation, 'sessionMetadata'> | null | undefined,
): Record<string, AutoFixSeen> {
  const raw = conversation?.sessionMetadata?.[AUTO_FIX_KEY]
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const state: Record<string, AutoFixSeen> = {}
  for (const [key, value] of Object.entries(raw)) {
    const seen = value as Partial<AutoFixSeen> | null
    if (!seen || typeof seen.url !== 'string') continue
    state[key] = {
      url: seen.url,
      failing: Array.isArray(seen.failing)
        ? seen.failing.filter((name) => typeof name === 'string')
        : [],
      reviews: typeof seen.reviews === 'number' ? seen.reviews : 0,
      conflict: seen.conflict === true,
      reports: typeof seen.reports === 'number' ? seen.reports : 0,
    }
  }
  return state
}

/** A PR's state as auto-fix reads it. */
export interface PrActivity {
  open: boolean
  failing: string[]
  reviews: number
  conflict: boolean
}

function parseJson<T>(text: string | undefined): T | null {
  if (!text?.trim()) return null
  try {
    return JSON.parse(text) as T
  } catch {
    return null
  }
}

/** The login `gh` acts as: its own reviews are never news. */
export async function ghViewer(gh: GhRunner): Promise<string | null> {
  const out = await gh(['api', 'user', '--jq', '.login'])
  return out?.exitCode === 0 ? out.stdout.trim() || null : null
}

/**
 * Two reads: the PR (state, conflict, reviews) and its checks. Issue
 * comments are left out: on most repositories they are bots reposting
 * status on every push. Null when gh could not answer.
 */
export async function readPrActivity(
  gh: GhRunner,
  pr: PullRequestRef,
  viewer: string | null,
): Promise<PrActivity | null> {
  const target = [String(pr.number), '-R', pr.repo]
  const [view, checks] = await Promise.all([
    gh(['pr', 'view', ...target, '--json', 'state,mergeable,reviews']),
    gh(['pr', 'checks', ...target, '--json', 'name,bucket']),
  ])
  const data = parseJson<{
    state?: string
    mergeable?: string
    reviews?: { author?: { login?: string }; state?: string }[]
  }>(view?.stdout)
  if (!data) return null
  const failing = (
    parseJson<{ name?: string; bucket?: string }[]>(checks?.stdout) ?? []
  )
    .filter((check) => check.bucket === 'fail' || check.bucket === 'cancel')
    .map((check) => check.name ?? '')
    .filter(Boolean)
  return {
    open: data.state?.toUpperCase() === 'OPEN',
    failing: [...new Set(failing)].sort(),
    reviews: (data.reviews ?? []).filter(
      (review) =>
        review.author?.login !== viewer &&
        (review.state === 'COMMENTED' || review.state === 'CHANGES_REQUESTED'),
    ).length,
    conflict: data.mergeable === 'CONFLICTING',
  }
}

/** What is new on a PR since auto-fix last reported, in words. */
export function autoFixNews(seen: AutoFixSeen, now: PrActivity): string[] {
  const news: string[] = []
  const failing = now.failing.filter((name) => !seen.failing.includes(name))
  if (failing.length > 0) {
    news.push(
      `${failing.length === 1 ? 'a check fails' : `${failing.length} checks fail`}: ${failing.join(', ')}`,
    )
  }
  const reviews = now.reviews - seen.reviews
  if (reviews > 0) {
    news.push(reviews === 1 ? 'a new review' : `${reviews} new reviews`)
  }
  if (now.conflict && !seen.conflict) news.push('it conflicts with its base')
  return news
}

/** The message that wakes the agent. */
export function autoFixMessage(url: string, news: readonly string[]): string {
  return `Auto-fix for ${url}: ${news.join('; ')}. Fix it and push to the PR's branch; reply on the review threads you address. Ask me before anything beyond the fix (force-push, settings, secrets).`
}

/** Where auto-fix starts on a PR: today's failures count as news. */
export function autoFixBaseline(
  pr: PullRequestRef,
  now: PrActivity | null,
): AutoFixSeen {
  return {
    url: pr.url,
    failing: [],
    reviews: now?.reviews ?? 0,
    conflict: false,
    reports: 0,
  }
}

/** `repo#number` back into a PR reference. */
function refOf(key: string, url: string): PullRequestRef | null {
  const match = /^(.+)#(\d+)$/.exec(key)
  return match ? { repo: match[1], number: Number(match[2]), url } : null
}

/**
 * One round over the PRs a chat watches: the state to keep (a PR that
 * closed or merged, or hit the cap, leaves it) and the message to send,
 * null when nothing is new. A PR gh cannot read keeps its last state.
 */
export async function autoFixRound(
  gh: GhRunner,
  state: Readonly<Record<string, AutoFixSeen>>,
  viewer: string | null,
): Promise<{ next: Record<string, AutoFixSeen>; message: string | null }> {
  const next: Record<string, AutoFixSeen> = { ...state }
  const messages: string[] = []
  for (const [key, seen] of Object.entries(state)) {
    const pr = refOf(key, seen.url)
    if (!pr) {
      delete next[key]
      continue
    }
    const now = await readPrActivity(gh, pr, viewer)
    if (!now) continue
    if (!now.open) {
      delete next[key]
      continue
    }
    const news = autoFixNews(seen, now)
    const reports = seen.reports + (news.length > 0 ? 1 : 0)
    next[key] = {
      url: seen.url,
      failing: now.failing,
      reviews: now.reviews,
      conflict: now.conflict,
      reports,
    }
    if (news.length > 0) messages.push(autoFixMessage(pr.url, news))
    if (reports >= AUTO_FIX_REPORT_CAP) delete next[key]
  }
  return {
    next,
    message: messages.length > 0 ? messages.join('\n\n') : null,
  }
}
