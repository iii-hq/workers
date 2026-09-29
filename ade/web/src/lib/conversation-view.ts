/**
 * The sidebar's list view: which session kinds it shows, how the roots are
 * grouped, and how they are ordered. One persisted record and one "Clear
 * filters" that puts every setting back to its default.
 *
 * The kind filter judges ROOT rows. A spawned sub-agent is a session of its
 * own, but it belongs to the chat that spawned it, so it follows its root's
 * kind (walking `parentId`) rather than its own value: the harness does not
 * stamp a kind on children, and a child must never surface as an orphan
 * "user" chat because its e2e parent was hidden.
 */

import {
  CONVERSATION_KINDS,
  type Conversation,
  type ConversationKind,
  isConversationKind,
} from '@/types/chat'
import {
  CONVERSATION_GROUPING_OPTIONS,
  type ConversationGrouping,
  DEFAULT_CONVERSATION_GROUPING,
  isConversationGrouping,
} from './conversation-groups'
import type { ConvNode } from './conversation-tree'

export type ConversationSort = 'activity' | 'created' | 'title'

export const DEFAULT_CONVERSATION_SORT: ConversationSort = 'activity'

export function isConversationSort(value: unknown): value is ConversationSort {
  return value === 'activity' || value === 'created' || value === 'title'
}

export interface ConversationListView {
  /** Kinds shown, in `CONVERSATION_KINDS` order. Empty hides every chat. */
  kinds: ConversationKind[]
  grouping: ConversationGrouping
  sort: ConversationSort
}

export const DEFAULT_CONVERSATION_LIST_VIEW: ConversationListView = {
  kinds: ['user'],
  grouping: DEFAULT_CONVERSATION_GROUPING,
  sort: DEFAULT_CONVERSATION_SORT,
}

/** The kinds the Type submenu offers, in menu order. */
export const CONVERSATION_KIND_OPTIONS: readonly {
  value: ConversationKind
  label: string
}[] = [
  { value: 'user', label: 'User' },
  { value: 'automation', label: 'Automation' },
  { value: 'e2e', label: 'E2E' },
]

/** The orders the Sort by submenu offers, in menu order. */
export const CONVERSATION_SORT_OPTIONS: readonly {
  value: ConversationSort
  label: string
}[] = [
  { value: 'activity', label: 'Last activity' },
  { value: 'created', label: 'Created' },
  { value: 'title', label: 'Title' },
]

/**
 * The mono tag a row wears when the list mixes kinds. A user chat wears
 * none: it is the default, and tagging it would tag almost every row.
 */
export const CONVERSATION_KIND_TAGS: Readonly<
  Partial<Record<ConversationKind, { label: string; title: string }>>
> = {
  automation: { label: 'auto', title: 'Automation run' },
  e2e: { label: 'e2e', title: 'End-to-end suite session' },
}

/** Dedupe and put a kind list in canonical order, dropping unknown values. */
export function normalizeKinds(kinds: Iterable<unknown>): ConversationKind[] {
  const wanted = new Set<unknown>(kinds)
  return CONVERSATION_KINDS.filter((kind) => wanted.has(kind))
}

export function isDefaultConversationListView(
  view: ConversationListView,
): boolean {
  const defaults = DEFAULT_CONVERSATION_LIST_VIEW
  return (
    view.grouping === defaults.grouping &&
    view.sort === defaults.sort &&
    view.kinds.length === defaults.kinds.length &&
    view.kinds.every((kind, i) => kind === defaults.kinds[i])
  )
}

export function toggleKind(
  view: ConversationListView,
  kind: ConversationKind,
  checked: boolean,
): ConversationListView {
  const next = new Set(view.kinds)
  if (checked) next.add(kind)
  else next.delete(kind)
  return { ...view, kinds: normalizeKinds(next) }
}

/**
 * Rebuild a view from whatever was persisted. Each field falls back to its
 * default on its own, so a stale or partial record never blanks the list.
 * `legacyGrouping` is the value of the older grouping-only key, honoured
 * only when the record itself has none.
 */
export function parseConversationListView(
  raw: unknown,
  legacyGrouping?: unknown,
): ConversationListView {
  const record =
    typeof raw === 'object' && raw !== null
      ? (raw as Record<string, unknown>)
      : {}
  const grouping = isConversationGrouping(record.grouping)
    ? record.grouping
    : isConversationGrouping(legacyGrouping)
      ? legacyGrouping
      : DEFAULT_CONVERSATION_GROUPING
  return {
    kinds: Array.isArray(record.kinds)
      ? normalizeKinds(record.kinds)
      : [...DEFAULT_CONVERSATION_LIST_VIEW.kinds],
    grouping,
    sort: isConversationSort(record.sort)
      ? record.sort
      : DEFAULT_CONVERSATION_SORT,
  }
}

/** What the Type row shows: `User`, `User, E2E`, `All`, or `None`. */
export function kindsLabel(kinds: readonly ConversationKind[]): string {
  if (kinds.length === 0) return 'None'
  if (kinds.length === CONVERSATION_KINDS.length) return 'All'
  return CONVERSATION_KIND_OPTIONS.filter((option) =>
    kinds.includes(option.value),
  )
    .map((option) => option.label)
    .join(', ')
}

/** The trigger tooltip: one line naming every setting. */
export function describeConversationListView(
  view: ConversationListView,
): string {
  const grouping =
    CONVERSATION_GROUPING_OPTIONS.find((o) => o.value === view.grouping)
      ?.label ?? view.grouping
  const sort =
    CONVERSATION_SORT_OPTIONS.find((o) => o.value === view.sort)?.label ??
    view.sort
  return `${kindsLabel(view.kinds)} · ${grouping} · ${sort}`
}

/**
 * The kind a row is listed under: its root's. Walks `parentId` through
 * `byId`; a parent that is not loaded ends the walk (the child is then a
 * root of its own, as the tree builder treats it), and a cycle stops at the
 * first repeat.
 */
export function effectiveConversationKind(
  conversation: Conversation,
  byId: ReadonlyMap<string, Conversation>,
): ConversationKind {
  const seen = new Set<string>([conversation.id])
  let current = conversation
  for (;;) {
    const parent = current.parentId ? byId.get(current.parentId) : undefined
    if (!parent || seen.has(parent.id)) break
    seen.add(parent.id)
    current = parent
  }
  return isConversationKind(current.kind) ? current.kind : 'user'
}

/** The conversations whose root's kind is one of `kinds`; order preserved. */
export function filterConversationsByKind(
  conversations: Conversation[],
  kinds: readonly ConversationKind[],
): Conversation[] {
  if (kinds.length === CONVERSATION_KINDS.length) return conversations
  const byId = new Map(conversations.map((c) => [c.id, c]))
  return conversations.filter((c) =>
    kinds.includes(effectiveConversationKind(c, byId)),
  )
}

const collator = new Intl.Collator(undefined, {
  sensitivity: 'base',
  numeric: true,
})

export function compareConversations(
  a: Conversation,
  b: Conversation,
  sort: ConversationSort,
): number {
  switch (sort) {
    case 'created':
      return b.createdAt - a.createdAt || b.updatedAt - a.updatedAt
    case 'title':
      return collator.compare(a.title, b.title) || b.updatedAt - a.updatedAt
    default:
      return b.updatedAt - a.updatedAt
  }
}

/**
 * Order root rows. Children are left alone: a sub-agent list is a spawn
 * log, and the tree already keeps it in creation order.
 */
export function sortConversationRoots(
  roots: ConvNode[],
  sort: ConversationSort,
): ConvNode[] {
  return [...roots].sort((a, b) =>
    compareConversations(a.conversation, b.conversation, sort),
  )
}
