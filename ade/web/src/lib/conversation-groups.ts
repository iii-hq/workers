/**
 * Sidebar sections for the conversation tree.
 *
 * Two modes answer two different questions. `recent` ("what was I just
 * doing?") buckets by calendar day — Today, Yesterday, Previous 7 days,
 * Previous 30 days, Older. `project` ("what was I doing *here*?") buckets by
 * the session's filesystem scope, `Conversation.workingDir`. `none` is the
 * flat list: one section the sidebar draws without a header.
 *
 * Grouping applies to ROOT rows only. A spawned sub-agent stays nested under
 * its parent, so a subtree never splits across two sections and a child
 * inherits its root's section by construction.
 *
 * Group keys carry their mode, so one persisted collapsed-set covers both
 * modes without a project folder named `today` colliding with the Today
 * bucket.
 *
 * On an engine with chat projects (`harness::projects`, chat-projects
 * §1.3.1) `project` groups by the conversation's project instead
 * (`metadata.project_id`): one section per project, ordered by its newest
 * conversation, an empty project kept as its header, and "No project" last.
 * Each project shows its 3 newest roots, then "N more" (`capProjectRoots`).
 */

import type { ChatProject } from './backend/projects'
import type { ConvNode } from './conversation-tree'

export type ConversationGrouping = 'none' | 'recent' | 'project'

export const DEFAULT_CONVERSATION_GROUPING: ConversationGrouping = 'recent'

export function isConversationGrouping(
  value: unknown,
): value is ConversationGrouping {
  return value === 'none' || value === 'recent' || value === 'project'
}

/** The groupings the filter menu offers, in menu order. */
export const CONVERSATION_GROUPING_OPTIONS: readonly {
  value: ConversationGrouping
  label: string
}[] = [
  { value: 'none', label: 'None' },
  { value: 'recent', label: 'Date' },
  { value: 'project', label: 'Project' },
]

export interface ConversationGroup {
  /** Mode-prefixed and stable across renders: the collapse-persistence key. */
  key: string
  /** Natural-case section label. */
  label: string
  /** Full path behind a shortened project label; the header tooltip. */
  title?: string
  /** Root nodes of this section, in the order `buildConversationTree` gave. */
  roots: ConvNode[]
  /** A chat project's section: the project it lists. */
  chatProject?: ChatProject
  /** The chat projects' "No project" section. */
  noProject?: boolean
}

/** How many roots a project shows before "N more" (§1.3.1). */
export const PROJECT_ROOTS_SHOWN = 3

export const CHAT_PROJECT_KEY_PREFIX = 'chat-project:'
export const CHAT_NO_PROJECT_GROUP_KEY = 'chat-project:none'

const DAY_MS = 86_400_000

/** Ordered narrowest-first; the first bucket a row fits wins. */
const RECENCY_BUCKETS = [
  { key: 'recent:today', label: 'Today', maxDays: 0 },
  { key: 'recent:yesterday', label: 'Yesterday', maxDays: 1 },
  { key: 'recent:7d', label: 'Previous 7 days', maxDays: 7 },
  { key: 'recent:30d', label: 'Previous 30 days', maxDays: 30 },
  {
    key: 'recent:older',
    label: 'Older',
    maxDays: Number.POSITIVE_INFINITY,
  },
] as const

export const NO_PROJECT_GROUP_KEY = 'project:none'
/** Chats the reader pinned sit above every grouping, in pin order. */
export const PINNED_GROUP_KEY = 'pinned'

/** The single section of the flat list; never drawn as a header. */
export const UNGROUPED_KEY = 'none:all'

function startOfDay(ts: number): number {
  const d = new Date(ts)
  d.setHours(0, 0, 0, 0)
  return d.getTime()
}

/**
 * Whole calendar days between two instants. Both ends are snapped to local
 * midnight and the quotient is rounded, so a 23h or 25h DST day cannot push a
 * row into the neighbouring bucket.
 */
export function calendarDaysAgo(ts: number, now: number): number {
  return Math.round((startOfDay(now) - startOfDay(ts)) / DAY_MS)
}

function recencyBucket(ts: number, now: number) {
  /* A timestamp ahead of the clock (skew between the engine and this tab)
     folds into Today rather than inventing a future section. */
  const days = Math.max(0, calendarDaysAgo(ts, now))
  return (
    RECENCY_BUCKETS.find((bucket) => days <= bucket.maxDays) ??
    RECENCY_BUCKETS[RECENCY_BUCKETS.length - 1]
  )
}

/** Trailing separators dropped so `/a/b` and `/a/b/` are one project. */
export function normalizeProjectPath(
  dir: string | null | undefined,
): string | null {
  const trimmed = (dir ?? '').trim()
  if (!trimmed) return null
  return trimmed.replace(/[/\\]+$/, '') || '/'
}

function segments(path: string): string[] {
  return path.split(/[/\\]+/).filter(Boolean)
}

/** Last path segment — enough to name a project in a 272px rail. */
function baseName(path: string): string {
  const parts = segments(path)
  return parts[parts.length - 1] ?? path
}

/** `parent/base`, the disambiguation for two projects with the same folder. */
function qualifiedName(path: string): string {
  const parts = segments(path)
  if (parts.length < 2) return baseName(path)
  return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`
}

function groupByRecency(roots: ConvNode[], now: number): ConversationGroup[] {
  const byKey = new Map<string, ConvNode[]>()
  for (const node of roots) {
    const { key } = recencyBucket(node.conversation.updatedAt, now)
    const bucket = byKey.get(key)
    if (bucket) bucket.push(node)
    else byKey.set(key, [node])
  }
  /* Emitted in bucket order, not insertion order, and empty ones are dropped
     so a quiet week shows no hollow sections. */
  return RECENCY_BUCKETS.flatMap(({ key, label }) => {
    const nodes = byKey.get(key)
    return nodes ? [{ key, label, roots: nodes }] : []
  })
}

function groupByProject(roots: ConvNode[]): ConversationGroup[] {
  const byPath = new Map<string, ConvNode[]>()
  const unscoped: ConvNode[] = []
  for (const node of roots) {
    const path = normalizeProjectPath(node.conversation.workingDir)
    if (!path) {
      unscoped.push(node)
      continue
    }
    const bucket = byPath.get(path)
    if (bucket) bucket.push(node)
    else byPath.set(path, [node])
  }

  /* Map insertion order is the roots' order, which is newest-first: a project
     sits where its most recent conversation does. */
  const baseNameCounts = new Map<string, number>()
  for (const path of byPath.keys()) {
    const base = baseName(path)
    baseNameCounts.set(base, (baseNameCounts.get(base) ?? 0) + 1)
  }

  const groups: ConversationGroup[] = []
  for (const [path, nodes] of byPath) {
    const base = baseName(path)
    groups.push({
      key: `project:${path}`,
      label: (baseNameCounts.get(base) ?? 0) > 1 ? qualifiedName(path) : base,
      title: path,
      roots: nodes,
    })
  }

  /* The catch-all sits last: it is where a chat lands by omission, not a
     project of its own. */
  if (unscoped.length > 0) {
    groups.push({
      key: NO_PROJECT_GROUP_KEY,
      label: 'No project',
      roots: unscoped,
    })
  }
  return groups
}

/**
 * Sections by chat project: a project's roots are the ones whose
 * `projectId` names it. `none`, an unknown id and no id at all land in
 * "No project", last and only when it has rows. Projects order by their
 * newest conversation; one without conversations sorts by its own
 * `updated_at_ms` and still shows (its header and "No conversations yet").
 */
export function groupByChatProject(
  roots: ConvNode[],
  projects: readonly ChatProject[],
): ConversationGroup[] {
  const byId = new Map(projects.map((project) => [project.id, project]))
  const rows = new Map<string, ConvNode[]>()
  const loose: ConvNode[] = []
  for (const node of roots) {
    const id = node.conversation.projectId
    if (id && byId.has(id)) {
      const bucket = rows.get(id)
      if (bucket) bucket.push(node)
      else rows.set(id, [node])
    } else {
      loose.push(node)
    }
  }
  const newest = (project: ChatProject): number => {
    const nodes = rows.get(project.id)
    if (!nodes?.length) return project.updated_at_ms
    return Math.max(...nodes.map((node) => node.conversation.updatedAt))
  }
  const groups: ConversationGroup[] = [...projects]
    .map((project) => ({ project, at: newest(project) }))
    .sort((a, b) => b.at - a.at || a.project.name.localeCompare(b.project.name))
    .map(({ project }) => ({
      key: CHAT_PROJECT_KEY_PREFIX + project.id,
      label: project.name,
      roots: rows.get(project.id) ?? [],
      chatProject: project,
    }))
  if (loose.length > 0) {
    groups.push({
      key: CHAT_NO_PROJECT_GROUP_KEY,
      label: 'No project',
      roots: loose,
      noProject: true,
    })
  }
  return groups
}

/**
 * The roots a project shows: the first 3, then "N more" until it is
 * expanded. The open conversation always shows, so the list never hides
 * where the reader is.
 */
export function capProjectRoots(
  roots: ConvNode[],
  expanded: boolean,
  activeId: string | null = null,
): { shown: ConvNode[]; hidden: number } {
  if (expanded || roots.length <= PROJECT_ROOTS_SHOWN) {
    return { shown: roots, hidden: 0 }
  }
  const shown = roots.slice(0, PROJECT_ROOTS_SHOWN)
  const active = activeId
    ? roots
        .slice(PROJECT_ROOTS_SHOWN)
        .find((node) => containsConversation(node, activeId))
    : undefined
  if (active) shown.push(active)
  return { shown, hidden: roots.length - shown.length }
}

function containsConversation(node: ConvNode, id: string): boolean {
  return (
    node.conversation.id === id ||
    node.children.some((child) => containsConversation(child, id))
  )
}

export function groupConversationRoots(
  roots: ConvNode[],
  grouping: ConversationGrouping,
  now: number,
  pinned: ReadonlySet<string> = new Set(),
  /** The engine's chat projects; `project` then groups by them, not by folder. */
  chatProjects?: readonly ChatProject[],
): ConversationGroup[] {
  const pinnedRoots: ConvNode[] = []
  const rest: ConvNode[] = []
  for (const node of roots) {
    ;(pinned.has(node.conversation.id) ? pinnedRoots : rest).push(node)
  }
  // The set iterates in pin order, which the reader arranges by dragging.
  const pinOrder = new Map([...pinned].map((id, index) => [id, index]))
  pinnedRoots.sort(
    (a, b) =>
      (pinOrder.get(a.conversation.id) ?? 0) -
      (pinOrder.get(b.conversation.id) ?? 0),
  )
  const head: ConversationGroup[] =
    pinnedRoots.length > 0
      ? [{ key: PINNED_GROUP_KEY, label: 'Pinned', roots: pinnedRoots }]
      : []
  if (grouping === 'none') {
    return rest.length > 0
      ? [
          ...head,
          { key: UNGROUPED_KEY, label: 'All conversations', roots: rest },
        ]
      : head
  }
  return [
    ...head,
    ...(grouping === 'project'
      ? chatProjects
        ? groupByChatProject(rest, chatProjects)
        : groupByProject(rest)
      : groupByRecency(rest, now)),
  ]
}

/** Every session a section holds, sub-agents included. */
export function countConversations(nodes: ConvNode[]): number {
  let total = 0
  for (const node of nodes) total += 1 + countConversations(node.children)
  return total
}
