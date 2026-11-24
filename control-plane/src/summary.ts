import { z } from 'zod'
import schema from './result.schema.json' with { type: 'json' }
import type { ControlPlaneData, NeedsYouItem } from './types.js'

export const SUMMARY_INTERVAL = 10 * 60_000
export const SUMMARY_MAX_AGE = 30 * 60_000
export const SUMMARY_SCHEMA = schema
/** Detail reads only: the wake's digest carries the scope, so the writer never lists a whole board. */
export const WRITER_READS = [
  'kanban::ticket::get',
  'kanban::activity::list',
  'session::messages-tail',
  'harness::status',
  'harness::ask::list',
  'github::pr::checks',
] as const

const refs = z
  .array(z.string().regex(new RegExp(schema.definitions.refs.items.pattern)))
  .refine((values) => new Set(values).size === values.length)
const line = z.strictObject({ text: z.string().min(1), refs: refs.min(1) })
const timestamp = z.iso
  .datetime()
  .regex(new RegExp(schema.definitions.timestamp.pattern))
const resultContract = z.strictObject({
  checked_at: timestamp,
  unchanged: z.boolean(),
  brief: z
    .strictObject({ headline: z.string(), happened: z.string(), happening: z.string(), remaining: z.string(), refs })
    .refine((brief) =>
      brief.headline
        ? brief.refs.length > 0 && Boolean(brief.happened || brief.happening || brief.remaining)
        : !brief.happened && !brief.happening && !brief.remaining && brief.refs.length === 0,
    ),
  diary: z
    .array(line.extend({ at: timestamp }))
    .max(schema.properties.diary.maxItems),
  now: z.record(
    z
      .string()
      .regex(
        new RegExp(Object.keys(schema.properties.now.patternProperties)[0]),
      ),
    line,
  ),
  pr_waits: z.record(
    z
      .string()
      .regex(
        new RegExp(
          Object.keys(schema.properties.pr_waits.patternProperties)[0],
        ),
      ),
    line,
  ),
  needs_you: z
    .array(
      z.strictObject({
        id: z.string().regex(new RegExp(schema.properties.needs_you.items.properties.id.pattern)),
        waiting: z.boolean(),
        text: z.string().min(1),
        recommendation: z.string(),
        refs: refs.min(1),
      }),
    )
    .max(schema.properties.needs_you.maxItems),
})

export type ControlPlaneSummary = z.infer<typeof resultContract>

export function acceptSummary(
  value: unknown,
  previous: ControlPlaneSummary | null,
  now = Date.now(),
): ControlPlaneSummary | null {
  const parsed = resultContract.safeParse(value)
  if (!parsed.success) return previous
  // The diary's order and duplicates are the worker's to fix, not a reason
  // to throw a whole summary away: newest first, each event once.
  const seen = new Set<string>()
  const next = {
    ...parsed.data,
    diary: [...parsed.data.diary]
      .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
      .filter(({ at, text }) => {
        const key = JSON.stringify([at, text])
        if (seen.has(key)) return false
        seen.add(key)
        return true
      }),
  }
  const checked = Date.parse(next.checked_at)
  if (
    checked > now ||
    (previous && checked < Date.parse(previous.checked_at)) ||
    next.diary.some((entry) => Date.parse(entry.at) > checked) ||
    [...Object.entries(next.now), ...Object.entries(next.pr_waits)].some(
      ([subject, note]) => !note.refs.includes(subject),
    )
  )
    return previous
  return next.unchanged && previous
    ? { ...previous, checked_at: next.checked_at, unchanged: true }
    : next
}

export function summaryIsFresh(checkedAt: number, now = Date.now()): boolean {
  return checkedAt <= now && now - checkedAt <= SUMMARY_MAX_AGE
}

export function resolveSummary(
  summary: ControlPlaneSummary,
  data: Pick<ControlPlaneData, 'tickets' | 'runs' | 'prs'>,
): ControlPlaneSummary {
  const sessions = new Set(
    data.runs.map((run) => `session:${run.session.session_id}`),
  )
  const prs = new Set(data.prs.map((pr) => `${pr.repo}#${pr.number}`))
  const live = new Set([
    ...data.tickets.map((card) => card.key),
    ...sessions,
    ...prs,
  ])
  const resolved = <T extends { refs: string[] }>(note: T): T => ({
    ...note,
    refs: note.refs.filter((ref) => live.has(ref)),
  })
  return {
    ...summary,
    brief: resolved(summary.brief),
    diary: summary.diary.map(resolved),
    now: Object.fromEntries(
      Object.entries(summary.now)
        .filter(([subject]) => sessions.has(subject))
        .map(([subject, note]) => [subject, resolved(note)]),
    ),
    pr_waits: Object.fromEntries(
      Object.entries(summary.pr_waits)
        .filter(([subject]) => prs.has(subject))
        .map(([subject, note]) => [subject, resolved(note)]),
    ),
    needs_you: summary.needs_you.map(resolved),
  }
}

type Item = ControlPlaneData['items'][number]

/**
 * Apply the writer's checks to the rule-based items: each gets the agent's note
 * and recommendation, a chat that only looked like a question is dropped, and
 * decisions the writer found that no rule covers are added (no rush, never
 * counted twice). Formal questions, stalled runs and marked cards always stay.
 */
export function withAgentChecks(
  items: ControlPlaneData['items'],
  summary: ControlPlaneSummary | null,
  data: Pick<ControlPlaneData, 'tickets' | 'runs' | 'board'>,
): ControlPlaneData['items'] {
  if (!summary) return items
  const checks = new Map(summary.needs_you.map((check) => [check.id, check]))
  const agent = (check: ControlPlaneSummary['needs_you'][number]) => ({
    waiting: check.waiting,
    text: check.text,
    recommendation: check.recommendation,
    checked_at: summary.checked_at,
  })
  const kept: Item[] = items.flatMap((item) => {
    const check = checks.get(item.id)
    if (!check) return [item]
    if (item.kind === 'reply' && !check.waiting) return []
    return [{ ...item, agent: agent(check) } as Item]
  })
  const covered = new Set(
    kept.flatMap((item) => [
      ...(item.card ? [item.card.key] : []),
      ...(item.sessionId ? [`session:${item.sessionId}`] : []),
    ]),
  )
  const closed = new Set([data.board?.doneStatus, data.board?.canceledStatus].filter(Boolean))
  const found: Item[] = summary.needs_you.flatMap((check) => {
    if (!check.id.startsWith('found:') || !check.waiting) return []
    const subject = check.id.slice('found:'.length)
    if (covered.has(subject)) return []
    const run = data.runs.find((row) => `session:${row.session.session_id}` === subject)
    const card = data.tickets.find((ticket) => ticket.key === subject && !closed.has(ticket.status)) ?? run?.card
    if (!run && !card) return []
    covered.add(subject)
    const need: NeedsYouItem & { urgency: 'no-rush' } = {
      id: check.id,
      kind: 'found',
      sessionId: run?.session.session_id ?? '',
      title: check.text,
      card,
      blockedCards: 0,
      createdAt: Date.parse(summary.checked_at),
      agent: agent(check),
      urgency: 'no-rush',
    }
    return [need]
  })
  return [...kept, ...found]
}
