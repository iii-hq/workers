/**
 * A board card as the List item its template binds to (design §5): every
 * string and every on/off the card shows, computed here (a template has no
 * `Show`, and the host formats nothing). Pure: no `iii`.
 */

import type { Agent, Summary } from './api'
import { isChangedSinceVisit, type MatchContext } from './filters'
import { ago, hueOf, pillIcon, priorityLabel, priorityRank } from './format'

export interface CardRow {
  /** The ticket's id: the List key, and what a drop names. */
  key: string
  status: string
  ticketKey: string
  title: string
  parent: string
  hasParent: boolean
  l0: string
  l1: string
  l2: string
  hasL0: boolean
  hasL1: boolean
  hasL2: boolean
  sel0: boolean
  sel1: boolean
  sel2: boolean
  more: string
  hasMore: boolean
  p0: string
  p1: string
  p0Icon: string
  p1Icon: string
  hasP0: boolean
  hasP1: boolean
  hasChips: boolean
  priority: string
  priorityVariant: string
  hasPriority: boolean
  changed: boolean
  changedAt: string
  subs: string
  hasSubs: boolean
  subsWarn: boolean
  comments: string
  hasComments: boolean
  assigneeIcon: string
  assigneeHue: string
  selected: boolean
}

export interface CardContext {
  match: MatchContext
  agents: ReadonlyMap<string, Agent>
  /** Ticket id → key, for the parent line (a parent the app does not hold is not named). */
  keys: ReadonlyMap<string, string>
  /** Parent id → its children's counts; null while the board is partial (the counts would be short). */
  subs: ReadonlyMap<string, { done: number; total: number }> | null
  /** Labels the filters ask for: first, and selected. */
  highlight: readonly string[]
  priorities: readonly string[]
  done: string
  selected: string | null
}

export function cardRow(t: Summary, ctx: CardContext): CardRow {
  const labels = [...new Set(t.labels)].sort(
    (a, b) => Number(ctx.highlight.includes(b)) - Number(ctx.highlight.includes(a)),
  )
  const label = (i: number) => labels[i] ?? ''
  const pills = t.pills.slice(0, 2)
  const rank = priorityRank(t.priority, ctx.priorities)
  const parent = t.parent_id ? ctx.keys.get(t.parent_id) : undefined
  const subs = ctx.subs?.get(t.id)
  const open = subs && t.status === ctx.done ? subs.total - subs.done : 0
  const changed = isChangedSinceVisit(t, ctx.match)
  const agent = t.assignee ? ctx.agents.get(t.assignee) : undefined
  return {
    key: t.id,
    status: t.status,
    ticketKey: t.key,
    title: t.title || t.key,
    parent: parent ? `Subtask of ${parent}` : '',
    hasParent: !!parent,
    l0: label(0),
    l1: label(1),
    l2: label(2),
    hasL0: labels.length > 0,
    hasL1: labels.length > 1,
    hasL2: labels.length > 2,
    sel0: ctx.highlight.includes(label(0)),
    sel1: ctx.highlight.includes(label(1)),
    sel2: ctx.highlight.includes(label(2)),
    more: labels.length > 3 ? `+${labels.length - 3}` : '',
    hasMore: labels.length > 3,
    p0: pills[0]?.text ?? '',
    p1: pills[1]?.text ?? '',
    p0Icon: pillIcon(pills[0]?.icon ?? ''),
    p1Icon: pillIcon(pills[1]?.icon ?? ''),
    hasP0: pills.length > 0,
    hasP1: pills.length > 1,
    hasChips: labels.length > 0 || pills.length > 0,
    priority: rank >= 0 ? priorityLabel(t.priority) : '',
    priorityVariant: rank === 0 ? 'alert' : 'warn',
    hasPriority: rank >= 0,
    changed,
    changedAt: changed ? ago(t.updated_at, ctx.match.now) : '',
    subs: subs ? `${subs.done}/${subs.total}` : '',
    hasSubs: !!subs,
    subsWarn: open > 0,
    comments: String(t.comment_count),
    hasComments: t.comment_count > 0,
    assigneeIcon: t.assignee ? 'bot' : 'circle-dashed',
    assigneeHue: hueOf(agent?.color),
    selected: ctx.selected === t.key,
  }
}
