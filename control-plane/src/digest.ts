import type { ControlPlaneData } from './types.js'

/** One board-wide `kanban::activity::list` row, as the writer's digest reads it. */
export interface ActivityRow {
  ticket_id: string
  kind: string
  actor: string
  at: string
  changes?: Array<{ field: string; from?: unknown; to?: unknown }> | null
}

const CARDS = 40
const ACTIVITY = 60
const RUNS = 20
const NEEDS = 20
const PRS = 30

const clip = (text: string, max: number) => {
  const line =
    text
      .split('\n')
      .find((part) => part.trim())
      ?.trim() ?? ''
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}
const short = (value: unknown) =>
  value === null || value === undefined
    ? ''
    : clip(typeof value === 'string' ? value : JSON.stringify(value), 60)

/**
 * The writer's bounded view of a scope, built from the worker's own read model.
 * A large board (hundreds of cards, megabytes of tickets) is too big for the
 * writer to list itself; this digest gives it what a summary needs in a few KB.
 */
export function writerDigest(
  data: ControlPlaneData,
  {
    scopeName,
    since,
    activity,
  }: { scopeName: string; since: string | null; activity: ActivityRow[] },
) {
  const sinceMs = since ? Date.parse(since) : 0
  const tickets = data.tickets ?? []
  const byId = new Map(tickets.map((ticket) => [ticket.id, ticket]))
  const card = (ticket: ControlPlaneData['tickets'][number]) => ({
    key: ticket.key,
    title: clip(ticket.title ?? '', 120),
    status: ticket.status,
    labels: ticket.labels ?? [],
    pills: (ticket.pills ?? []).map((pill) => pill.text),
    updated_at: ticket.updated_at,
  })
  const newest = [...tickets].sort((a, b) => (b.updated_at ?? '').localeCompare(a.updated_at ?? ''))
  const done = data.board?.doneStatus
  return {
    scope: scopeName,
    board: data.board
      ? {
          done: tickets.filter((ticket) => ticket.status === done).length,
          total: tickets.length,
          columns: data.board.columns.map((column) => ({
            id: column.id,
            label: column.label,
            total: column.total,
          })),
        }
      : null,
    releases: (data.releases ?? []).map((release) => ({
      name: release.name,
      status: release.status,
      done: release.done,
      total: release.total,
      open_questions: release.open_questions,
    })),
    active_cards: newest
      .filter(
        (ticket) =>
          ticket.status === 'in_progress' ||
          ticket.status === 'in_review' ||
          (ticket.labels ?? []).includes('blocked'),
      )
      .slice(0, CARDS)
      .map(card),
    changed_cards: newest
      .filter((ticket) => Date.parse(ticket.updated_at ?? '') > sinceMs)
      .slice(0, CARDS)
      .map(card),
    activity: activity
      .filter((row) => byId.has(row.ticket_id))
      .slice(0, ACTIVITY)
      .map((row) => ({
        at: row.at,
        card: byId.get(row.ticket_id)?.key,
        kind: row.kind,
        actor: row.actor,
        changes: (row.changes ?? [])
          .slice(0, 3)
          .map((change) => `${change.field}: ${short(change.from)} → ${short(change.to)}`),
      })),
    runs: (data.runs ?? []).slice(0, RUNS).map((run) => ({
      ref: `session:${run.session.session_id}`,
      title: clip(run.session.title ?? '', 120),
      card: run.card?.key ?? null,
      state: run.state,
      last_progress: Number.isFinite(run.lastProgress)
        ? new Date(run.lastProgress).toISOString()
        : null,
      last_message: clip(run.lastAssistant ?? '', 300),
    })),
    needs_you: (data.items ?? []).slice(0, NEEDS).map((item) => ({
      kind: item.kind,
      urgency: item.urgency,
      class: item.kind === 'question' ? item.questionClass : undefined,
      card: item.card?.key ?? null,
      session: `session:${item.sessionId}`,
      title: clip(item.title ?? '', 200),
    })),
    // null: pull requests were not read this time (no repositories, GitHub unavailable, or not read yet).
    prs: !(data.repositories ?? []).length
      ? null
      : (data.prs ?? []).slice(0, PRS).map((pr) => ({
          ref: `${pr.repo}#${pr.number}`,
          title: clip(pr.title ?? '', 120),
          card: pr.card?.key ?? null,
          checks: pr.checks,
          draft: pr.isDraft,
          wait: pr.waitReason,
        })),
  }
}
