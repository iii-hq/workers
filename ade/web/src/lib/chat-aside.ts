import { createContext } from 'react'
import type { IiiClient } from '@/lib/iii-client'
import type { Conversation, Message } from '@/types/chat'

/**
 * Data behind the chat's side panel: which sessions are side chats, the
 * caps on long sections, links and pull requests named in the transcript,
 * and the git and GitHub reads for the Context tab. Pure parsing is
 * exported for tests; the reads go through `shell::exec` (argv `git`, as the
 * IDE does) and the optional `github` worker.
 */

/** Session metadata key naming the conversation a side chat forked from. */
export const SIDE_CHAT_KEY = 'side_chat_of'

/**
 * Set by a chat that has a side panel: asks about `text` in its side chat.
 * Messages read it to offer "Ask in side chat"; absent, they offer nothing.
 */
export const AskInSideChatContext = createContext<
  ((text: string) => void) | null
>(null)

export function sideChatOf(
  conversation: Pick<Conversation, 'sessionMetadata'>,
): string | null {
  const value = conversation.sessionMetadata?.[SIDE_CHAT_KEY]
  return typeof value === 'string' && value !== '' ? value : null
}

export function isSideChat(
  conversation: Pick<Conversation, 'sessionMetadata'>,
): boolean {
  return sideChatOf(conversation) !== null
}

/** Side chats this browser tab created: the only ones it discards on leave. */
export const sideChatsMadeHere = new Set<string>()

/**
 * Keys the harness writes on a spawned session. A fork copies them, and a
 * side chat must not keep them: deleting it would then notify the parent
 * (waking the orchestrator), and its approvals would route to the parent.
 */
const SPAWN_LINK_KEYS = [
  'parent_session_id',
  'parent_turn_id',
  'function_call_id',
  'parent_function_call_id',
  'depth',
  'spawned_by',
  'root_session_id',
]

/** The fork's metadata as a side chat of `mainId`: tagged, unlinked from any parent. */
export function sideChatMetadata(
  source: Record<string, unknown> | undefined,
  mainId: string,
): Record<string, unknown> {
  const metadata: Record<string, unknown> = { ...(source ?? {}) }
  for (const key of SPAWN_LINK_KEYS) delete metadata[key]
  metadata[SIDE_CHAT_KEY] = mainId
  return metadata
}

/** Conversations to list: no side chats, and nothing spawned inside one. */
export function hideSideChats<T extends Conversation>(
  conversations: readonly T[],
): T[] {
  const byId = new Map(conversations.map((c) => [c.id, c]))
  const hidden = (conversation: T): boolean => {
    const seen = new Set<string>()
    let current: T | undefined = conversation
    while (current && !seen.has(current.id)) {
      if (isSideChat(current)) return true
      seen.add(current.id)
      current = current.parentId ? byId.get(current.parentId) : undefined
    }
    return false
  }
  return conversations.filter((c) => !hidden(c))
}

/** The newest side chat still attached to `mainId`. */
export function findSideChat(
  conversations: readonly Conversation[],
  mainId: string,
): Conversation | null {
  let found: Conversation | null = null
  for (const conversation of conversations) {
    if (sideChatOf(conversation) !== mainId) continue
    if (!found || (conversation.createdAt ?? 0) > (found.createdAt ?? 0)) {
      found = conversation
    }
  }
  return found
}

/**
 * A fork copies the history with its timestamps: what came after is the side
 * chat's own. ponytail: `since` is the server's clock and a not-yet-echoed
 * message carries the browser's; skew only misfiles a message sent within
 * that skew of the fork. Anchor on the copied leaf entry if that ever bites.
 */
export function sideChatMessages(
  messages: readonly Message[],
  since: number,
): Message[] {
  return messages.filter((message) => message.createdAt >= since)
}

const QUOTE_LIMIT = 600

/** A markdown blockquote of `text`, ready for a question under it. */
export function sideChatQuote(text: string): string {
  const trimmed = text.trim()
  const cut =
    trimmed.length > QUOTE_LIMIT
      ? `${trimmed.slice(0, QUOTE_LIMIT).trimEnd()}…`
      : trimmed
  const quoted = cut
    .split('\n')
    .map((line) => `> ${line}`.trimEnd())
    .join('\n')
  return `${quoted}\n\n`
}

/**
 * The rows a section shows: `cap` of them, or all when expanded, or when the
 * only row left would be the one a "Show 1 more" button would cost.
 */
export function capRows<T>(
  rows: readonly T[],
  cap: number,
  expanded: boolean,
): { shown: readonly T[]; hidden: number } {
  if (expanded || rows.length <= cap + 1) return { shown: rows, hidden: 0 }
  return { shown: rows.slice(0, cap), hidden: rows.length - cap }
}

export interface PullRequestRef {
  /** `owner/name`. */
  repo: string
  number: number
  url: string
}

export interface ChatLink {
  url: string
  host: string
  /** The path without its leading slash, or the host for a bare origin. */
  label: string
}

const PR_URL = /^https:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/pull\/(\d+)/
const URL_IN_TEXT = /https?:\/\/[^\s<>()[\]{}"'`|\\]+/g
const TRAILING_PUNCTUATION = /[.,;:!?*_~]+$/

export function parsePullRequestUrl(url: string): PullRequestRef | null {
  const match = PR_URL.exec(url)
  if (!match) return null
  const number = Number(match[2])
  if (!Number.isSafeInteger(number)) return null
  // GitHub names are case-insensitive; one spelling keeps one row.
  const repo = match[1].toLowerCase()
  return { repo, number, url: `https://github.com/${repo}/pull/${number}` }
}

function decodePath(path: string): string {
  try {
    return decodeURIComponent(path)
  } catch {
    // `%s` in a format string is not an escape; show the path as written.
    return path
  }
}

/** URLs found in one message, cached by message: a stream only rescans its own. */
const urlsByMessage = new WeakMap<Message, string[]>()

function messageUrls(message: Message): string[] {
  const cached = urlsByMessage.get(message)
  if (cached) return cached
  const urls = (messageText(message).match(URL_IN_TEXT) ?? []).map((raw) =>
    raw.replace(TRAILING_PUNCTUATION, ''),
  )
  urlsByMessage.set(message, urls)
  return urls
}

function messageText(message: Message): string {
  if (message.role === 'user' || message.role === 'assistant') {
    return message.content
  }
  // A PR the agent opened is in the call's result, not in its prose.
  if (
    message.role === 'function-trigger' &&
    message.functionId === 'github::pr::create'
  ) {
    try {
      return JSON.stringify(message.output ?? '')
    } catch {
      return ''
    }
  }
  return ''
}

/** URLs the conversation names, newest first: pull requests apart from the rest. */
export function collectChatLinks(messages: readonly Message[]): {
  links: ChatLink[]
  pullRequests: PullRequestRef[]
} {
  const links: ChatLink[] = []
  const pullRequests: PullRequestRef[] = []
  const seen = new Set<string>()
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    for (const url of messageUrls(messages[index])) {
      const pr = parsePullRequestUrl(url)
      const key = pr ? `${pr.repo}#${pr.number}` : url
      if (seen.has(key)) continue
      seen.add(key)
      if (pr) {
        pullRequests.push(pr)
        continue
      }
      let parsed: URL
      try {
        parsed = new URL(url)
      } catch {
        continue
      }
      const path = decodePath(parsed.pathname).replace(/^\/+|\/+$/g, '')
      links.push({ url, host: parsed.host, label: path || parsed.host })
    }
  }
  return { links, pullRequests }
}

/** `owner/name` from a GitHub remote URL (https, scp-style or ssh://). */
export function parseGithubRemote(remote: string): string | null {
  const match =
    /^(?:https:\/\/(?:[^@/]+@)?|ssh:\/\/git@|git@)github\.com(?::\d+)?[:/]([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/.exec(
      remote.trim(),
    )
  return match ? `${match[1]}/${match[2]}`.toLowerCase() : null
}

export interface GitSummary {
  branch: string | null
  /** Commits ahead of the upstream; 0 without one. */
  ahead: number
  /** Files `git status` lists, untracked included. */
  changed: number
  /** Lines added and removed against HEAD (tracked files). */
  added: number
  removed: number
  /** `owner/name` of the GitHub `origin`, when there is one. */
  repo: string | null
}

/** Branch and ahead count from `git status --branch`'s `## ` header. */
export function parseStatusHeader(header: string): {
  branch: string | null
  ahead: number
} {
  const body = header.replace(/^## /, '')
  if (body.startsWith('HEAD (no branch)')) return { branch: null, ahead: 0 }
  const name = body
    .replace(/^(No commits yet|Initial commit) on /, '')
    .split(/\.\.\.| /)[0]
  const ahead = /\[ahead (\d+)/.exec(body)
  return { branch: name || null, ahead: ahead ? Number(ahead[1]) : 0 }
}

export function parseNumstat(stdout: string): {
  added: number
  removed: number
} {
  let added = 0
  let removed = 0
  for (const line of stdout.split('\n')) {
    const [plus, minus] = line.split('\t')
    // Binary files report `-`; they change no lines.
    if (/^\d+$/.test(plus ?? '')) added += Number(plus)
    if (/^\d+$/.test(minus ?? '')) removed += Number(minus)
  }
  return { added, removed }
}

interface ExecResult {
  exit_code: number | null
  stdout: string
}

type Trigger = IiiClient['trigger']

/** One read of the folder's git state; `null` when it is not a repository. */
export async function readGitSummary(
  trigger: Trigger,
  cwd: string,
): Promise<GitSummary | null> {
  const git = (args: string[]) =>
    trigger<ExecResult>('shell::exec', {
      command: 'git',
      // fsmonitor off: a repository's own config must not run a hook here.
      args: ['--no-optional-locks', '-c', 'core.fsmonitor=false', ...args],
      cwd,
      timeout_ms: 15_000,
    })
  const [status, diff, remote] = await Promise.allSettled([
    // `normal` counts an untracked folder once instead of every file in it.
    git(['status', '--porcelain=v1', '--branch', '--untracked-files=normal']),
    git(['diff', 'HEAD', '--numstat']),
    git(['remote', 'get-url', 'origin']),
  ])
  if (status.status !== 'fulfilled' || status.value.exit_code !== 0) return null
  const lines = status.value.stdout.split('\n').filter(Boolean)
  const header = lines[0]?.startsWith('## ') ? lines[0] : ''
  const { branch, ahead } = parseStatusHeader(header)
  const lineCounts =
    diff.status === 'fulfilled' && diff.value.exit_code === 0
      ? parseNumstat(diff.value.stdout)
      : { added: 0, removed: 0 }
  return {
    branch,
    ahead,
    changed: lines.length - (header ? 1 : 0),
    ...lineCounts,
    repo:
      remote.status === 'fulfilled' && remote.value.exit_code === 0
        ? parseGithubRemote(remote.value.stdout)
        : null,
  }
}

export type PullRequestState = 'open' | 'draft' | 'merged' | 'closed'
export type ChecksState = 'pass' | 'fail' | 'pending'

export interface PullRequestRow extends PullRequestRef {
  title: string
  state: PullRequestState
  checks: ChecksState | null
  /** Named in the conversation rather than found by the branch. */
  mentioned: boolean
}

interface GhPullRequest {
  number: number
  title: string
  url: string
  state: string
  isDraft?: boolean
  headRefName?: string
}

/**
 * Branches whose `head:` search only finds strangers' PRs (forks opened from
 * their own default branch). ponytail: a name list; read `origin/HEAD` if a
 * repository's default is something else.
 */
const DEFAULT_BRANCHES = new Set(['main', 'master'])

function prState(pr: GhPullRequest): PullRequestState {
  const state = pr.state.toUpperCase()
  if (state === 'MERGED') return 'merged'
  if (state === 'CLOSED') return 'closed'
  return pr.isDraft ? 'draft' : 'open'
}

/** One word for a PR's checks, from `gh pr checks`' buckets. */
export function summarizeChecks(
  checks: readonly { bucket?: string }[],
): ChecksState | null {
  if (checks.length === 0) return null
  if (checks.some((c) => c.bucket === 'fail' || c.bucket === 'cancel'))
    return 'fail'
  if (checks.some((c) => c.bucket === 'pending')) return 'pending'
  return 'pass'
}

const MENTIONED_PR_LIMIT = 3

/**
 * The branch's pull requests (`head:<branch>` search) and up to three the
 * conversation names, open ones first, each with its checks while open.
 */
export async function readPullRequests(
  trigger: Trigger,
  opts: {
    repo: string | null
    branch: string | null
    mentioned: readonly PullRequestRef[]
  },
): Promise<PullRequestRow[]> {
  const rows: PullRequestRow[] = []
  const add = (repo: string, pr: GhPullRequest, mentioned: boolean) => {
    if (rows.some((row) => row.repo === repo && row.number === pr.number))
      return
    rows.push({
      repo,
      number: pr.number,
      url: pr.url,
      title: pr.title,
      state: prState(pr),
      checks: null,
      mentioned,
    })
  }
  const branch = opts.branch
  if (opts.repo && branch && !DEFAULT_BRANCHES.has(branch)) {
    const listed = await trigger<{ value?: GhPullRequest[] }>(
      'github::pr::list',
      {
        repo: opts.repo,
        state: 'all',
        search: `head:${branch}`,
        limit: 5,
      },
    ).catch(() => null)
    // `head:` matches by prefix; keep the branch's own PRs only.
    for (const pr of listed?.value ?? []) {
      if (pr.headRefName === undefined || pr.headRefName === branch) {
        add(opts.repo, pr, false)
      }
    }
  }
  const mentioned = opts.mentioned
    .filter(
      (ref) =>
        !rows.some((row) => row.repo === ref.repo && row.number === ref.number),
    )
    .slice(0, MENTIONED_PR_LIMIT)
  const viewed = await Promise.all(
    mentioned.map((ref) =>
      trigger<{ value?: GhPullRequest }>('github::pr::view', {
        repo: ref.repo,
        number: ref.number,
      }).catch(() => null),
    ),
  )
  viewed.forEach((view, index) => {
    if (view?.value) add(mentioned[index].repo, view.value, true)
  })
  await Promise.all(
    rows
      .filter((row) => row.state === 'open' || row.state === 'draft')
      .map(async (row) => {
        const checks = await trigger<{ value?: { bucket?: string }[] }>(
          'github::pr::checks',
          { repo: row.repo, number: row.number },
        ).catch(() => null)
        row.checks = Array.isArray(checks?.value)
          ? summarizeChecks(checks.value)
          : null
      }),
  )
  const rank = (row: PullRequestRow) =>
    row.state === 'open' || row.state === 'draft' ? 0 : 1
  return rows.sort((a, b) => rank(a) - rank(b))
}
