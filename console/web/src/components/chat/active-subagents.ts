import type { IIIConnectionState } from '@/lib/iii-client'
import type {
  Conversation,
  SubagentAppearance,
  SubagentColor,
  SubagentIcon,
} from '@/types/chat'

export const SUBAGENT_ICONS = [
  'agent',
  'code',
  'search',
  'terminal',
  'database',
  'test',
  'review',
  'docs',
  'design',
] as const satisfies readonly SubagentIcon[]

export const SUBAGENT_COLORS = [
  'neutral',
  'blue',
  'purple',
  'teal',
  'green',
  'amber',
  'rose',
] as const satisfies readonly SubagentColor[]

export type SubagentVisualStatus =
  | 'queued'
  | 'working'
  | 'waiting'
  | 'completed'
  | 'failed'
  | 'stopped'
  | 'disconnected'

export type ActiveSubagentStatus = Extract<
  SubagentVisualStatus,
  'queued' | 'working' | 'waiting' | 'disconnected'
>

export interface ResolvedSubagentAppearance {
  name: string
  icon: SubagentIcon
  color: SubagentColor
}

export interface SubagentViewModel {
  sessionId: string
  appearance: ResolvedSubagentAppearance
  status: SubagentVisualStatus
  relativeDepth: number
  conversation: Conversation
}

export interface ActiveSubagentViewModel
  extends Omit<SubagentViewModel, 'status'> {
  status: ActiveSubagentStatus
}

export interface TerminalSubagentSummary {
  completed: number
  failed: number
  stopped: number
  total: number
}

export interface ActiveSubagentChipModel {
  active: ActiveSubagentViewModel[]
  terminal: TerminalSubagentSummary
  omittedActive: number
  /** The source list, depth, or descendant budget was exhausted. */
  truncated: boolean
}

export interface SubagentCollectionOptions {
  maxConversations?: number
  maxDescendants?: number
  maxDepth?: number
}

export interface ActiveSubagentChipModelOptions
  extends SubagentCollectionOptions {
  maxVisible?: number
}

export const DEFAULT_MAX_SUBAGENT_CONVERSATIONS = 512
// session::list currently contributes at most 200 rows; 512 leaves headroom
// for live-created descendants without letting old terminal children hide a
// newer active one before status filtering.
export const DEFAULT_MAX_SUBAGENT_DESCENDANTS = 512
export const DEFAULT_MAX_SUBAGENT_DEPTH = 16
// Keep every normally collected active descendant actionable. Callers may
// still opt into a smaller presentation, and the traversal ceilings protect
// the composer from hostile or corrupt session graphs.
export const DEFAULT_MAX_VISIBLE_SUBAGENTS = DEFAULT_MAX_SUBAGENT_DESCENDANTS

const HARD_MAX_CONVERSATIONS = 2_048
const HARD_MAX_DESCENDANTS = HARD_MAX_CONVERSATIONS
const HARD_MAX_DEPTH = 32
const HARD_MAX_VISIBLE = HARD_MAX_DESCENDANTS
const MAX_APPEARANCE_NAME_LENGTH = 48
const MAX_STOP_MARKER_MESSAGES = 32

const ICON_SET = new Set<string>(SUBAGENT_ICONS)
const COLOR_SET = new Set<string>(SUBAGENT_COLORS)
const STOPPED_REASON = /\b(?:stopping|stopped|cancelled|canceled|aborted)\b/i
const QUEUED_REASON =
  /\b(?:queue|queued|scheduled|dispatching|pending dispatch|waiting to start)\b/i
const WAITING_REASON = /\b(?:waiting|awaiting|blocked|paused|approval)\b/i

interface CollectedSubagents {
  descendants: Array<{ conversation: Conversation; relativeDepth: number }>
  truncated: boolean
}

/**
 * Breadth-first descendant traversal with hard ceilings at every dimension.
 * Session metadata is external input, so `visited` also makes corrupt cyclic
 * parent links harmless.
 */
export function collectSubagentDescendants(
  conversations: readonly Conversation[],
  rootSessionId: string,
  options: SubagentCollectionOptions = {},
): CollectedSubagents {
  if (!rootSessionId) return { descendants: [], truncated: false }

  const maxConversations = boundedLimit(
    options.maxConversations,
    DEFAULT_MAX_SUBAGENT_CONVERSATIONS,
    HARD_MAX_CONVERSATIONS,
  )
  const maxDescendants = boundedLimit(
    options.maxDescendants,
    DEFAULT_MAX_SUBAGENT_DESCENDANTS,
    HARD_MAX_DESCENDANTS,
  )
  const maxDepth = boundedLimit(
    options.maxDepth,
    DEFAULT_MAX_SUBAGENT_DEPTH,
    HARD_MAX_DEPTH,
  )
  const scannedCount = Math.min(conversations.length, maxConversations)
  const childrenByParent = new Map<string, Conversation[]>()

  for (let index = 0; index < scannedCount; index += 1) {
    const conversation = conversations[index]
    if (!conversation?.parentId) continue
    const siblings = childrenByParent.get(conversation.parentId)
    if (siblings) siblings.push(conversation)
    else childrenByParent.set(conversation.parentId, [conversation])
  }

  for (const children of childrenByParent.values()) {
    children.sort(compareConversations)
  }

  const queue: Array<{ conversation: Conversation; relativeDepth: number }> = (
    childrenByParent.get(rootSessionId) ?? []
  ).map((conversation) => ({
    conversation,
    relativeDepth: 1,
  }))
  const descendants: CollectedSubagents['descendants'] = []
  const visited = new Set<string>([rootSessionId])
  let cursor = 0
  let depthTruncated = false

  while (cursor < queue.length && descendants.length < maxDescendants) {
    const next = queue[cursor]
    cursor += 1
    const { conversation, relativeDepth } = next
    if (visited.has(conversation.id)) continue
    visited.add(conversation.id)
    descendants.push(next)

    const children = childrenByParent.get(conversation.id)
    if (!children?.length) continue
    if (relativeDepth >= maxDepth) {
      depthTruncated = true
      continue
    }
    for (const child of children) {
      if (!visited.has(child.id)) {
        queue.push({ conversation: child, relativeDepth: relativeDepth + 1 })
      }
    }
  }

  return {
    descendants,
    truncated:
      scannedCount < conversations.length ||
      cursor < queue.length ||
      depthTruncated,
  }
}

/** Derive the user-facing lifecycle without mutating the conversation. */
export function deriveSubagentVisualStatus(
  conversation: Conversation,
  connectionState: IIIConnectionState,
): SubagentVisualStatus {
  // Harness cancellations are persisted as done + stopped. An error remains
  // a failure even when its diagnostic happens to contain words like
  // "aborted" or "stopped".
  if (conversation.status === 'error') return 'failed'
  if (
    STOPPED_REASON.test(conversation.statusReason ?? '') ||
    (conversation.status === 'done' && hasStoppedMarker(conversation))
  ) {
    return 'stopped'
  }
  if (conversation.status === 'done') return 'completed'
  if (connectionState !== 'connected') return 'disconnected'

  const reason = conversation.statusReason ?? ''
  if (QUEUED_REASON.test(reason)) return 'queued'
  if (WAITING_REASON.test(reason)) return 'waiting'
  if (conversation.status === 'working') return 'working'

  // New child sessions exist briefly before their first message is accepted.
  // An idle child with transcript history is instead available for a wake.
  return conversation.messages.length === 0 ? 'queued' : 'waiting'
}

export function resolveSubagentAppearance(
  conversation: Conversation,
): ResolvedSubagentAppearance {
  const appearance = conversation.subagentAppearance as
    | SubagentAppearance
    | undefined
  const fallbackName =
    conversation.title && conversation.title !== conversation.id
      ? conversation.title
      : 'Sub-agent'
  const name = normalizedName(appearance?.name) ?? normalizedName(fallbackName)

  return {
    name: name ?? 'Sub-agent',
    icon:
      appearance?.icon && ICON_SET.has(appearance.icon)
        ? appearance.icon
        : 'agent',
    color:
      appearance?.color && COLOR_SET.has(appearance.color)
        ? appearance.color
        : 'neutral',
  }
}

/** Build the bounded model consumed by the composer-adjacent chip row. */
export function buildActiveSubagentChipModel(
  conversations: readonly Conversation[],
  rootSessionId: string,
  connectionState: IIIConnectionState,
  options: ActiveSubagentChipModelOptions = {},
): ActiveSubagentChipModel {
  const collected = collectSubagentDescendants(
    conversations,
    rootSessionId,
    options,
  )
  const allActive: ActiveSubagentViewModel[] = []
  const terminal: TerminalSubagentSummary = {
    completed: 0,
    failed: 0,
    stopped: 0,
    total: 0,
  }

  for (const descendant of collected.descendants) {
    const { conversation, relativeDepth } = descendant
    const status = deriveSubagentVisualStatus(conversation, connectionState)
    if (isActiveSubagentStatus(status)) {
      allActive.push({
        sessionId: conversation.id,
        appearance: resolveSubagentAppearance(conversation),
        status,
        relativeDepth,
        conversation,
      })
      continue
    }
    terminal[status] += 1
    terminal.total += 1
  }

  const maxVisible = boundedLimit(
    options.maxVisible,
    DEFAULT_MAX_VISIBLE_SUBAGENTS,
    HARD_MAX_VISIBLE,
  )
  return {
    active: allActive.slice(0, maxVisible),
    terminal,
    omittedActive: Math.max(0, allActive.length - maxVisible),
    truncated: collected.truncated,
  }
}

function hasStoppedMarker(conversation: Conversation): boolean {
  const start = Math.max(
    0,
    conversation.messages.length - MAX_STOP_MARKER_MESSAGES,
  )
  for (
    let index = conversation.messages.length - 1;
    index >= start;
    index -= 1
  ) {
    const message = conversation.messages[index]
    if (message.id.endsWith('_stopped')) return true
    if (message.role === 'assistant' && message.stopReason === 'aborted') {
      return true
    }
  }
  return false
}

function isActiveSubagentStatus(
  status: SubagentVisualStatus,
): status is ActiveSubagentStatus {
  return (
    status === 'queued' ||
    status === 'working' ||
    status === 'waiting' ||
    status === 'disconnected'
  )
}

function normalizedName(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const compact = value.trim().replace(/\s+/g, ' ')
  if (!compact) return null
  return Array.from(compact).slice(0, MAX_APPEARANCE_NAME_LENGTH).join('')
}

function boundedLimit(
  value: number | undefined,
  fallback: number,
  maximum: number,
): number {
  if (value === undefined || !Number.isFinite(value) || value < 1) {
    return fallback
  }
  return Math.min(Math.floor(value), maximum)
}

function compareConversations(left: Conversation, right: Conversation): number {
  return left.createdAt - right.createdAt || left.id.localeCompare(right.id)
}
