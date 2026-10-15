import { unwrapEnvelope } from '@iii-dev/console-ui/format'
import { createContext, type RefObject, useEffect } from 'react'
import type { IiiClient } from '@/lib/iii-client'
import { findMentions } from '@/lib/mentions/token'
import { belongsToEntry } from '@/lib/sessions/entry-mapper'
import type {
  Attachment,
  Conversation,
  FunctionTriggerMessage,
  Message,
} from '@/types/chat'

/**
 * Data behind the chat's side panel: which sessions are side chats, the
 * caps on long sections, links and pull requests named in the transcript,
 * and the git and GitHub reads for the Context tab. Pure parsing is
 * exported for tests; the reads go through `shell::exec` (argv `git`, as the
 * IDE does) and the optional `github` worker.
 */

/** Session metadata key naming the conversation a side chat forked from. */
export const SIDE_CHAT_KEY = 'side_chat_of'

/** A passage of a main-chat message asked about in the side chat. */
export interface SideChatQuote {
  /** The main-chat message it came from (the transcript row id). */
  messageId: string
  text: string
  /** Who wrote the message, for the card's label. */
  agent?: string
}

/**
 * Set by a chat that has a side panel: asks about a quote in its side chat,
 * and names the messages the side chat holds quotes of. Messages read it to
 * offer "Ask in side chat" and to mark themselves; absent, they do neither.
 */
export const AskInSideChatContext = createContext<{
  ask: (quote: SideChatQuote) => void
  quotedIds: ReadonlySet<string>
} | null>(null)

/** Set by a side chat: the quotes it sent, and the way back to their source. */
export const SideChatQuotesContext = createContext<{
  quotes: readonly SideChatQuote[]
  jump: (quote: SideChatQuote) => void
} | null>(null)

/** Session metadata key holding the quotes a side chat sent. */
export const SIDE_CHAT_QUOTES_KEY = 'side_chat_quotes'
const QUOTES_KEPT = 20

/** The quotes recorded on a side chat (malformed entries are skipped). */
export function sideChatQuotes(
  conversation: Pick<Conversation, 'sessionMetadata'> | null | undefined,
): SideChatQuote[] {
  const raw = conversation?.sessionMetadata?.[SIDE_CHAT_QUOTES_KEY]
  if (!Array.isArray(raw)) return []
  const quotes: SideChatQuote[] = []
  for (const entry of raw) {
    if (!entry || typeof entry !== 'object') continue
    const { message_id, text, agent } = entry as Record<string, unknown>
    if (typeof message_id !== 'string' || typeof text !== 'string') continue
    quotes.push({
      messageId: message_id,
      text,
      ...(typeof agent === 'string' ? { agent } : {}),
    })
  }
  return quotes
}

/** The metadata value after recording `quote`, newest last, bounded. */
export function withSideChatQuote(
  quotes: readonly SideChatQuote[],
  quote: SideChatQuote,
): Record<string, string>[] {
  return [...quotes, quote].slice(-QUOTES_KEPT).map((q) => ({
    message_id: q.messageId,
    text: q.text,
    ...(q.agent ? { agent: q.agent } : {}),
  }))
}

/**
 * A user message the side chat sent with a quote: the quote (its card) and
 * the question under it. Matches the exact blockquote `sideChatQuote` built.
 */
export function splitQuotedMessage(
  content: string,
  quotes: readonly SideChatQuote[],
): { quote: SideChatQuote; rest: string } | null {
  for (let index = quotes.length - 1; index >= 0; index -= 1) {
    const prefix = sideChatQuote(quotes[index].text)
    if (content.startsWith(prefix)) {
      return { quote: quotes[index], rest: content.slice(prefix.length) }
    }
  }
  return null
}

/**
 * The range of `text` inside `root`'s rendered text, ignoring whitespace (a
 * selection joins blocks with newlines the DOM does not have), or null.
 */
export function findTextRange(root: Node, text: string): Range | null {
  const wanted = text.replace(/\s+/g, '')
  if (!wanted) return null
  const chars: string[] = []
  const at: Array<{ node: Text; offset: number }> = []
  const walker = root.ownerDocument?.createTreeWalker(
    root,
    NodeFilter.SHOW_TEXT,
  )
  for (let node = walker?.nextNode(); node; node = walker?.nextNode()) {
    const value = (node as Text).data
    for (let offset = 0; offset < value.length; offset += 1) {
      if (/\s/.test(value[offset])) continue
      chars.push(value[offset])
      at.push({ node: node as Text, offset })
    }
  }
  const start = chars.join('').indexOf(wanted)
  if (start < 0 || !root.ownerDocument) return null
  const first = at[start]
  const last = at[start + wanted.length - 1]
  const range = root.ownerDocument.createRange()
  range.setStart(first.node, first.offset)
  range.setEnd(last.node, last.offset + 1)
  return range
}

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

const MADE_KEY = 'iii-side-chats'

function readMade(): string[] {
  try {
    const ids: unknown = JSON.parse(localStorage.getItem(MADE_KEY) ?? '[]')
    return Array.isArray(ids)
      ? ids.filter((id): id is string => typeof id === 'string')
      : []
  } catch {
    return []
  }
}

function writeMade(ids: readonly string[]): void {
  try {
    if (ids.length) localStorage.setItem(MADE_KEY, JSON.stringify(ids))
    else localStorage.removeItem(MADE_KEY)
  } catch {}
}

/**
 * Side chats this browser made and has not kept or discarded yet, in
 * localStorage: they end when every ADE tab of the browser is closed (see
 * `sideChatsLeftByClosedTabs`).
 */
export const sideChatsMadeHere = {
  has(id: string): boolean {
    return readMade().includes(id)
  },
  add(id: string): void {
    const made = readMade()
    // ponytail: capped, so ids that never show up again cannot pile up.
    if (!made.includes(id)) writeMade([...made, id].slice(-50))
  },
  delete(id: string): void {
    writeMade(readMade().filter((made) => made !== id))
  },
  clear(): void {
    writeMade([])
  },
}

const TAB_KEY = 'iii-ade-tab'
const INSTANCE_LOCK = 'iii-ade-instance'
let leftByClosedTabs: Promise<string[]> | null = null

/**
 * The side chats this browser made that outlived every ADE tab, asked once
 * per tab: non-empty only in the first tab opened after all were closed,
 * which ends them as the server's list shows them (each leaves the record
 * as it ends).
 * Each tab holds a shared Web Lock the browser drops when it closes, so the
 * lock list says whether another tab is open; a reload keeps the tab's
 * sessionStorage, so it is no new start. Nothing can be deleted reliably
 * while a tab closes, hence at the next start.
 */
export function sideChatsLeftByClosedTabs(): Promise<string[]> {
  leftByClosedTabs ??= (async () => {
    let reloaded = true
    try {
      reloaded = sessionStorage.getItem(TAB_KEY) !== null
      sessionStorage.setItem(TAB_KEY, '1')
    } catch {}
    const locks = typeof navigator === 'undefined' ? undefined : navigator.locks
    if (!locks) return []
    const { held = [] } = await locks.query()
    const others = held.some((lock) => lock.name === INSTANCE_LOCK)
    void locks.request(
      INSTANCE_LOCK,
      { mode: 'shared' },
      () => new Promise<void>(() => {}),
    )
    return reloaded || others ? [] : readMade()
  })().catch(() => [])
  return leftByClosedTabs
}

const ending = new Set<string>()

/** Delete a side chat once per tab, whichever panel asks first. */
export function endSideChat(
  id: string,
  remove: (id: string) => Promise<void>,
): void {
  if (ending.has(id)) return
  ending.add(id)
  sideChatsMadeHere.delete(id)
  void remove(id).catch(() => {
    ending.delete(id)
  })
}

/** How many of this tab's chat panels show each conversation. */
const displayedHere = new Map<string, number>()

/** Count `id` as shown by one more panel; the returned function undoes it. */
export function markDisplayed(id: string): () => void {
  displayedHere.set(id, (displayedHere.get(id) ?? 0) + 1)
  return () => {
    const left = (displayedHere.get(id) ?? 1) - 1
    if (left <= 0) displayedHere.delete(id)
    else displayedHere.set(id, left)
  }
}

/** Whether a panel of this tab still shows `id`. */
export function isDisplayedHere(id: string): boolean {
  return displayedHere.has(id)
}

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

/** Session metadata key naming the fork's copy of the entry it was cut at. */
export const SIDE_CHAT_FROM_KEY = 'side_chat_from'

/**
 * The fork's metadata as a side chat of `mainId`: tagged, unlinked from any
 * parent, and anchored on `fromEntry`, its last copied entry.
 */
export function sideChatMetadata(
  source: Record<string, unknown> | undefined,
  mainId: string,
  fromEntry?: string,
): Record<string, unknown> {
  const metadata: Record<string, unknown> = { ...(source ?? {}) }
  for (const key of SPAWN_LINK_KEYS) delete metadata[key]
  metadata[SIDE_CHAT_KEY] = mainId
  if (fromEntry) metadata[SIDE_CHAT_FROM_KEY] = fromEntry
  else delete metadata[SIDE_CHAT_FROM_KEY]
  return metadata
}

export function sideChatFrom(
  conversation: Pick<Conversation, 'sessionMetadata'> | null | undefined,
): string | null {
  const value = conversation?.sessionMetadata?.[SIDE_CHAT_FROM_KEY]
  return typeof value === 'string' && value !== '' ? value : null
}

/**
 * Where to cut a side chat: the last user or assistant entry of a
 * `session::messages-tail` page. Its copy is the fork's last entry, and a
 * spoken row always renders, so the side chat can anchor on it.
 */
export function lastSpokenEntry(
  items: readonly { entry_id: string; message?: { role?: string } }[],
): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const role = items[index].message?.role
    if (role === 'user' || role === 'assistant') return items[index].entry_id
  }
  return null
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

/**
 * The text selected inside one transcript row under `root`, as a side chat
 * quote with where it sits on screen; null for no selection, a selection
 * spanning rows, or one outside the transcript (the composer, a dialog).
 */
export function selectionQuote(
  root: HTMLElement | null,
  messages: readonly Message[],
  agentName?: string,
): { quote: SideChatQuote; rect: DOMRect } | null {
  const selection = root?.ownerDocument.getSelection()
  if (!root || !selection || selection.isCollapsed || !selection.rangeCount) {
    return null
  }
  const text = selection.toString()
  if (!text.trim()) return null
  const range = selection.getRangeAt(0)
  const rowOf = (node: Node) =>
    (node instanceof Element ? node : node.parentElement)?.closest(
      '[data-message-row]',
    ) ?? null
  const row = rowOf(range.startContainer)
  if (!row || row !== rowOf(range.endContainer) || !root.contains(row)) {
    return null
  }
  const messageId = row.getAttribute('data-message-row') ?? ''
  const source = messages.find((message) => message.id === messageId)
  return {
    quote: {
      messageId,
      text,
      agent:
        source?.role === 'assistant' ? source.agent || agentName : undefined,
    },
    rect: range.getBoundingClientRect(),
  }
}

const QUOTE_HIGHLIGHT = 'side-chat-quote'

/**
 * Mark each quoted passage inside its message row under `root`, through the
 * CSS Custom Highlight API: no DOM is touched, so React and the markdown
 * renderer never see it. Without the API the message's own marker remains.
 * `version` re-runs the search when the transcript re-renders.
 */
export function useQuoteHighlights(
  rootRef: RefObject<HTMLElement | null>,
  quotes: readonly SideChatQuote[],
  version: unknown,
): void {
  // biome-ignore lint/correctness/useExhaustiveDependencies: version is a re-run token for a re-rendered transcript
  useEffect(() => {
    const root = rootRef.current
    if (
      !root ||
      quotes.length === 0 ||
      typeof Highlight === 'undefined' ||
      typeof CSS === 'undefined' ||
      !CSS.highlights
    ) {
      return
    }
    let highlight = CSS.highlights.get(QUOTE_HIGHLIGHT)
    if (!highlight) {
      highlight = new Highlight()
      CSS.highlights.set(QUOTE_HIGHLIGHT, highlight)
    }
    const added: Range[] = []
    for (const quote of quotes) {
      const row = root.querySelector(
        `[data-message-row="${CSS.escape(quote.messageId)}"]`,
      )
      const range = row ? findTextRange(row, quote.text) : null
      if (!range) continue
      highlight.add(range)
      added.push(range)
    }
    return () => {
      for (const range of added) highlight?.delete(range)
    }
  }, [rootRef, quotes, version])
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
 * The side chat's own messages: what comes after `fromEntry`, the copy of
 * the entry the fork was cut at. Copied history renders before it, notes and
 * markers included: a transcript page gives those no time of their own (they
 * are stamped at load), so a time filter alone lets them through. With no
 * anchor in the loaded window (the side chat outgrew it, or a fork from
 * before anchors), fall back to the fork time, which every echoed message
 * carries.
 */
export function sideChatMessages(
  messages: readonly Message[],
  since: number,
  fromEntry?: string | null,
): Message[] {
  if (fromEntry) {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (belongsToEntry(messages[index].id, fromEntry)) {
        return messages.slice(index + 1)
      }
    }
  }
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
    // Files one by one, as the IDE's Source control (where the row leads)
    // counts them.
    git(['status', '--porcelain=v1', '--branch', '--untracked-files=all']),
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

export interface GhPullRequest {
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

/** Where pull requests are read: the `github` worker, or `gh` in the folder. */
export interface PullRequestSource {
  list(repo: string, branch: string): Promise<GhPullRequest[] | null>
  view(repo: string, number: number): Promise<GhPullRequest | null>
  checks(repo: string, number: number): Promise<{ bucket?: string }[] | null>
}

/** Pull requests through the optional `github` worker. */
export function githubWorkerSource(trigger: Trigger): PullRequestSource {
  const value = async <T>(fn: string, payload: Record<string, unknown>) =>
    (await trigger<{ value?: T }>(fn, payload).catch(() => null))?.value ?? null
  return {
    // `head:` matches by prefix and across forks; the caller filters.
    list: (repo, branch) =>
      value<GhPullRequest[]>('github::pr::list', {
        repo,
        state: 'all',
        search: `head:${branch}`,
        limit: 5,
      }),
    view: (repo, number) =>
      value<GhPullRequest>('github::pr::view', { repo, number }),
    checks: (repo, number) =>
      value<{ bucket?: string }[]>('github::pr::checks', { repo, number }),
  }
}

const PR_FIELDS = 'number,title,url,state,isDraft,headRefName'

/**
 * Pull requests through the `gh` CLI in the folder, over `shell::exec`, for a
 * stack without the `github` worker. Read-only commands; `gh` keeps its own
 * login. A failure (no `gh`, no login) reads as no pull requests.
 */
export function ghCliSource(trigger: Trigger, cwd: string): PullRequestSource {
  const gh = async <T>(args: string[]): Promise<T | null> => {
    const out = await trigger<ExecResult>('shell::exec', {
      command: 'gh',
      args,
      cwd,
      timeout_ms: 20_000,
    }).catch(() => null)
    // `gh pr checks` exits 1 or 8 on failing or pending checks: the JSON on
    // stdout is the answer either way.
    if (!out?.stdout?.trim()) return null
    try {
      return JSON.parse(out.stdout) as T
    } catch {
      return null
    }
  }
  return {
    list: (repo, branch) =>
      gh<GhPullRequest[]>([
        'pr',
        'list',
        '-R',
        repo,
        '--head',
        branch,
        '--state',
        'all',
        '--limit',
        '5',
        '--json',
        PR_FIELDS,
      ]),
    view: (repo, number) =>
      gh<GhPullRequest>([
        'pr',
        'view',
        String(number),
        '-R',
        repo,
        '--json',
        PR_FIELDS,
      ]),
    checks: (repo, number) =>
      gh<{ bucket?: string }[]>([
        'pr',
        'checks',
        String(number),
        '-R',
        repo,
        '--json',
        'bucket',
      ]),
  }
}

/**
 * The branch's pull requests and up to three the conversation names, open
 * ones first, each with its checks while open.
 */
export async function readPullRequests(
  source: PullRequestSource,
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
    const listed = await source.list(opts.repo, branch)
    // Keep the branch's own PRs only.
    for (const pr of listed ?? []) {
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
    mentioned.map((ref) => source.view(ref.repo, ref.number)),
  )
  viewed.forEach((view, index) => {
    if (view) add(mentioned[index].repo, view, true)
  })
  await Promise.all(
    rows
      .filter((row) => row.state === 'open' || row.state === 'draft')
      .map(async (row) => {
        const checks = await source.checks(row.repo, row.number)
        row.checks = Array.isArray(checks) ? summarizeChecks(checks) : null
      }),
  )
  const rank = (row: PullRequestRow) =>
    row.state === 'open' || row.state === 'draft' ? 0 : 1
  return rows.sort((a, b) => rank(a) - rank(b))
}

/** Images found in one message, cached by message like its links. */
const imagesByMessage = new WeakMap<Message, Attachment[]>()
const IMAGE_SCAN_NODES = 400

/**
 * Image blocks a tool returned (`{ type: 'image', data, mime }`, any
 * `mimeType`/`media_type` spelling) and `data:image/…` strings, as
 * attachments the viewer can show. A bounded walk: outputs can be large.
 */
function outputImages(message: FunctionTriggerMessage): Attachment[] {
  const found: Attachment[] = []
  let budget = IMAGE_SCAN_NODES
  const push = (dataUrl: string, type: string) =>
    found.push({
      id: `${message.id}:image-${found.length + 1}`,
      name: `${message.functionId} image ${found.length + 1}`,
      size: Math.floor(dataUrl.length * 0.75),
      type,
      dataUrl,
    })
  const walk = (value: unknown, depth: number) => {
    if (budget-- <= 0 || depth > 6 || value === null) return
    if (typeof value === 'string') {
      const match = /^data:(image\/[\w.+-]+);base64,/.exec(value)
      if (match) push(value, match[1])
      return
    }
    if (typeof value !== 'object') return
    if (Array.isArray(value)) {
      for (const item of value) walk(item, depth + 1)
      return
    }
    const record = value as Record<string, unknown>
    if (record.type === 'image' && typeof record.data === 'string') {
      const mime = [record.mime, record.mimeType, record.media_type].find(
        (candidate): candidate is string => typeof candidate === 'string',
      )
      const type = mime ?? 'image/png'
      push(
        record.data.startsWith('data:')
          ? record.data
          : `data:${type};base64,${record.data}`,
        type,
      )
      return
    }
    for (const item of Object.values(record)) walk(item, depth + 1)
  }
  walk(message.output, 0)
  return found
}

/** Pictures in the conversation, newest first: attached ones and tool output. */
export function collectChatImages(messages: readonly Message[]): Attachment[] {
  const images: Attachment[] = []
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    let found = imagesByMessage.get(message)
    if (!found) {
      found =
        message.role === 'user'
          ? (message.attachments ?? []).filter((a) =>
              a.type.startsWith('image/'),
            )
          : message.role === 'function-trigger'
            ? outputImages(message)
            : []
      imagesByMessage.set(message, found)
    }
    images.push(...found)
  }
  return images
}

// ── tickets ──

/** What this chat did to a ticket, read off its own kanban calls. */
export type TicketAction =
  | 'created'
  | 'updated'
  | 'moved'
  | 'commented'
  | 'deleted'
  | 'restored'

/** A ticket the chat touched: worked here, by a sub-agent, or mentioned. */
export interface TicketTouch {
  /** As the chat named it: a key (KAN-12) or the ticket's uuid. */
  ref: string
  /** This chat's own call on it, the latest. */
  action?: TicketAction
  /** The sub-agent that works it, by name. */
  by?: string
  at: number
}

/** Worked here beats by a sub-agent beats mentioned. */
const touchRank = (touch: TicketTouch) => (touch.action ? 2 : touch.by ? 1 : 0)

/** The call's argument naming the ticket (its output, for a create). */
const KANBAN_CALLS: Record<
  string,
  { action: TicketAction; ref: (input: unknown, output: unknown) => unknown }
> = {
  'kanban::ticket::create': {
    action: 'created',
    ref: (_, output) => field(output, 'key') ?? field(output, 'id'),
  },
  'kanban::ticket::update': { action: 'updated', ref: (i) => field(i, 'id') },
  'kanban::ticket::move': { action: 'moved', ref: (i) => field(i, 'id') },
  'kanban::comment::create': {
    action: 'commented',
    ref: (i) => field(i, 'ticket_id'),
  },
  'kanban::ticket::delete': { action: 'deleted', ref: (i) => field(i, 'id') },
  'kanban::ticket::restore': {
    action: 'restored',
    ref: (i) => field(i, 'id'),
  },
}

/** Ticket keys as people write them; the board drops look-alikes (UTF-8). */
const TICKET_KEY = /\b[A-Z][A-Z0-9]{0,9}-\d{1,6}\b/g

function asRecord(value: unknown): Record<string, unknown> | null {
  let parsed = value
  if (typeof parsed === 'string') {
    try {
      parsed = JSON.parse(parsed)
    } catch {
      return null
    }
  }
  parsed = unwrapEnvelope(parsed)
  return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
    ? (parsed as Record<string, unknown>)
    : null
}

function field(value: unknown, key: string): string | undefined {
  const found = asRecord(value)?.[key]
  return typeof found === 'string' && found ? found : undefined
}

function keysIn(text: string): string[] {
  return [...new Set(text.match(TICKET_KEY) ?? [])]
}

/** Tickets one message touches, cached by message like its links. */
const ticketsByMessage = new WeakMap<Message, TicketTouch[]>()

function messageTickets(message: Message): TicketTouch[] {
  const cached = ticketsByMessage.get(message)
  if (cached) return cached
  const at = message.createdAt
  const found: TicketTouch[] = []
  if (message.role === 'function-trigger') {
    const call = KANBAN_CALLS[message.functionId]
    const ref = call?.ref(message.input, message.output)
    if (call && typeof ref === 'string' && ref) {
      found.push({ ref, action: call.action, at })
    } else if (message.functionId === 'harness::spawn') {
      const input = asRecord(message.input)
      const by = field(input?.display, 'name') ?? 'Sub-agent'
      const task = input?.task
      const text = typeof task === 'string' ? task : JSON.stringify(task ?? '')
      for (const ref of keysIn(text)) found.push({ ref, by, at })
    }
  } else if (message.role === 'user' || message.role === 'assistant') {
    for (const ref of keysIn(message.content)) found.push({ ref, at })
    for (const mention of findMentions(message.content)) {
      if (mention.name === 'kanban') found.push({ ref: mention.id, at })
    }
  }
  ticketsByMessage.set(message, found)
  return found
}

/**
 * The tickets a chat touched, one per ref: its own kanban calls (the latest
 * wins), then sub-agents whose instruction or title names a ticket, then
 * keys and `@kanban` mentions in what was said. Which refs are tickets at
 * all is the board's call (see `ticketRows`).
 */
export function collectChatTickets(
  messages: readonly Message[],
  subagents: readonly { name: string; title: string; at: number }[] = [],
): TicketTouch[] {
  const touches = new Map<string, TicketTouch>()
  const add = (touch: TicketTouch) => {
    const known = touches.get(touch.ref)
    if (
      !known ||
      touchRank(touch) > touchRank(known) ||
      (touchRank(touch) === touchRank(known) && touch.at >= known.at)
    ) {
      touches.set(touch.ref, touch)
    }
  }
  for (const message of messages) messageTickets(message).forEach(add)
  for (const subagent of subagents) {
    for (const ref of keysIn(subagent.title)) {
      add({ ref, by: subagent.name, at: subagent.at })
    }
  }
  return [...touches.values()]
}

/** One ticket as the board lists it. */
export interface KanbanTicket {
  id: string
  key: string
  title: string
  status: string
  priority: string
  deletedAt: string | null
}

/** The board, enough to name what a chat touched. */
export interface KanbanBoard {
  /** Column id → its label. */
  columns: ReadonlyMap<string, string>
  /** By key and by uuid. */
  tickets: ReadonlyMap<string, KanbanTicket>
}

/** Two calls, whatever the chat touched: the columns and every ticket. */
export async function readKanbanBoard(
  trigger: IiiClient['trigger'],
): Promise<KanbanBoard> {
  const [info, list] = await Promise.all([
    trigger<{ columns?: { id: string; label: string }[] }>(
      'kanban::config::info',
      {},
    ),
    trigger<{ tickets?: Record<string, unknown>[] }>('kanban::ticket::list', {
      include_deleted: true,
    }),
  ])
  const columns = new Map(
    (info?.columns ?? []).map((column) => [column.id, column.label]),
  )
  const tickets = new Map<string, KanbanTicket>()
  for (const row of list?.tickets ?? []) {
    const ticket: KanbanTicket = {
      id: String(row.id ?? ''),
      key: String(row.key ?? ''),
      title: String(row.title ?? ''),
      status: String(row.status ?? ''),
      priority: String(row.priority ?? ''),
      deletedAt: typeof row.deleted_at === 'string' ? row.deleted_at : null,
    }
    if (!ticket.key) continue
    tickets.set(ticket.key, ticket)
    if (ticket.id) tickets.set(ticket.id, ticket)
  }
  return { columns, tickets }
}

/** A touched ticket the board knows, ready to list. */
export interface TicketRow extends TicketTouch {
  ticket: KanbanTicket
  /** The column's label. */
  status: string
}

/**
 * The touched tickets the board knows, in list order: worked ones (here or
 * by a sub-agent) newest first, then the mentioned, then the deleted.
 */
export function ticketRows(
  touches: readonly TicketTouch[],
  board: KanbanBoard,
): TicketRow[] {
  const rows = new Map<string, TicketRow>()
  for (const touch of touches) {
    const ticket = board.tickets.get(touch.ref)
    if (!ticket) continue
    const row: TicketRow = {
      ...touch,
      ticket,
      status: board.columns.get(ticket.status) ?? ticket.status,
    }
    // A uuid and a key can name the same ticket: keep the stronger touch.
    const known = rows.get(ticket.key)
    if (
      !known ||
      touchRank(row) > touchRank(known) ||
      (touchRank(row) === touchRank(known) && row.at > known.at)
    ) {
      rows.set(ticket.key, row)
    }
  }
  const group = (row: TicketRow) =>
    row.ticket.deletedAt ? 2 : touchRank(row) > 0 ? 0 : 1
  return [...rows.values()].sort((a, b) => group(a) - group(b) || b.at - a.at)
}
