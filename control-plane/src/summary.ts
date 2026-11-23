import { z } from 'zod'
import schema from './result.schema.json' with { type: 'json' }
import type { ControlPlaneData } from './types.js'

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
})

export type ControlPlaneSummary = z.infer<typeof resultContract>

export function acceptSummary(
  value: unknown,
  previous: ControlPlaneSummary | null,
  now = Date.now(),
): ControlPlaneSummary | null {
  const parsed = resultContract.safeParse(value)
  if (!parsed.success) return previous
  const next = parsed.data
  const checked = Date.parse(next.checked_at)
  if (
    checked > now ||
    (previous && checked < Date.parse(previous.checked_at)) ||
    next.diary.some((entry, index) => {
      const at = Date.parse(entry.at)
      return (
        at > checked || (index > 0 && at > Date.parse(next.diary[index - 1].at))
      )
    }) ||
    new Set(next.diary.map(({ at, text }) => JSON.stringify([at, text])))
      .size !== next.diary.length ||
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
  }
}
