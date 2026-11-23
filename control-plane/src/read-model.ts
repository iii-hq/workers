import { createHash } from 'node:crypto'
import type {
  ControlPlaneClient,
  SessionMeta,
  TranscriptItem,
  ControlPlaneData,
  ControlPlaneConfig,
  Direction,
} from './types.js'
import {
  ALL_PROJECTS,
  type AskEntry,
  type HarnessStatus,
  isInScope,
  isStalled,
  type KanbanQuestionView,
  linkedCard,
  prWaitReason,
  type ControlPlaneBoard,
  type ControlPlaneProgress,
  type ControlPlaneProject,
  type ControlPlanePullRequest,
  type ControlPlaneQuestion,
  type ControlPlaneRelease,
  type ControlPlaneRun,
  type ControlPlaneTicket,
} from './model.js'
interface BoardSnapshot {
  projects: ControlPlaneProject[]
  releases: ControlPlaneRelease[]
  board: ControlPlaneBoard
  tickets: ControlPlaneTicket[]
  releaseCatalog: ControlPlaneRelease[]
}

interface RunSnapshot {
  session: SessionMeta
  status: HarnessStatus
  subtasks: ControlPlaneTicket[] | null
  runningChildren: number
  childOpenAsks: number
  descendantSessionIds: string[]
  lastAssistant: string
  context_size: number
  completedTurnId?: string
}

export function isRootTurnEvent(payload: unknown): boolean {
  if (!payload || typeof payload !== 'object') return false
  const event = payload as Record<string, unknown>
  if (typeof event.depth === 'number') return event.depth === 0
  return event.parent == null && event.parent_session_id == null
}

function linkedTicket(session?: Pick<SessionMeta, 'metadata'>): string | undefined {
  const ticket = session?.metadata?.kanban_ticket
  return typeof ticket === 'string' && ticket ? ticket : undefined
}

function lastAssistant(items: TranscriptItem[]): string {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const message = items[index].message
    if (message?.role !== 'assistant') continue
    const text = message.content
      .filter((block) => block.type === 'text')
      .map((block) => block.text)
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim()
    if (text) return text
  }
  return ''
}

export async function fetchControlPlaneBoard(
  iii: ControlPlaneClient,
  scope: string,
): Promise<BoardSnapshot> {
  const [projectsResult, releasesResult, boardResult] = await Promise.all([
    iii.call<{ projects: ControlPlaneProject[] }>('kanban::project::list', {}),
    iii.call<{ releases: ControlPlaneRelease[] }>('kanban::release::list', {}),
    iii.call<{
      columns: ControlPlaneBoard['columns']
      done_status: string
      canceled_status?: string | null
      id_prefix: string
    }>('kanban::board::get', scope === ALL_PROJECTS ? {} : { project_id: scope }),
  ])
  const columns = boardResult.columns ?? []
  const catalog = releasesResult.releases ?? []
  const scopedReleases = catalog.flatMap((release) => {
    if (scope === ALL_PROJECTS) return [release]
    const project = release.projects.find((row) => row.project_id === scope)
    return project
      ? [
          {
            ...release,
            total: project.total,
            done: project.done,
            open_questions: project.open_questions,
          },
        ]
      : []
  })
  const released = scopedReleases
    .filter((release) => release.status === 'released')
    .sort((a, b) => (b.released_at ?? '').localeCompare(a.released_at ?? ''))
    .slice(0, 2)
  const active = scopedReleases.filter((release) => release.status === 'active')
  const planned = scopedReleases
    .filter((release) => release.status === 'planned')
    .sort((a, b) => (a.target_at ?? '9999').localeCompare(b.target_at ?? '9999'))
  return {
    projects: projectsResult.projects ?? [],
    releases: [...released, ...active, ...planned],
    releaseCatalog: scopedReleases,
    tickets: columns.flatMap((column) => column.tickets),
    board: {
      columns,
      doneStatus: boardResult.done_status,
      canceledStatus: boardResult.canceled_status,
      idPrefix: boardResult.id_prefix,
    },
  }
}

export async function fetchControlPlaneAsks(
  iii: ControlPlaneClient,
  sessionId?: string,
): Promise<AskEntry[]> {
  const entries: AskEntry[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{ entries: AskEntry[]; cursor?: string }>('harness::ask::list', {
      limit: 200,
      ...(sessionId ? { session_id: sessionId } : {}),
      ...(cursor ? { cursor } : {}),
    })
    entries.push(...(page.entries ?? []))
    cursor = page.cursor
  } while (cursor)
  return entries
}

export async function fetchControlPlaneQuestions(
  iii: ControlPlaneClient,
  scope: string,
): Promise<KanbanQuestionView[]> {
  const questions: KanbanQuestionView[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{
      questions: KanbanQuestionView[]
      cursor?: string
    }>('kanban::question::list', {
      limit: 500,
      ...(scope === ALL_PROJECTS ? {} : { project: scope }),
      ...(cursor ? { cursor } : {}),
    })
    questions.push(...(page.questions ?? []))
    cursor = page.cursor
  } while (cursor)
  return questions
}

function questionKey(sessionId: string, questionId: string): string {
  return `${sessionId}/${questionId}`
}

/** Merge the board decision log with the harness's authoritative lifecycle. */
export function mergeControlPlaneQuestions(
  asks: readonly AskEntry[],
  boardRows: readonly KanbanQuestionView[],
): ControlPlaneQuestion[] {
  const merged = new Map<string, ControlPlaneQuestion>()
  for (const board of boardRows) {
    const question = board.comment.question
    if (!question) continue
    const key = questionKey(question.session_id, question.question_id)
    merged.set(key, {
      key,
      sessionId: question.session_id,
      questionId: question.question_id,
      ticketId: board.comment.ticket_id,
      commentId: board.comment.id,
      questions: question.questions,
      about: question.about ?? {},
      askedAt: question.asked_at,
      state: board.state,
      answerText: board.answer?.answer?.answer_text,
      by: board.answer?.answer?.by,
      outcome: board.answer?.answer?.outcome,
      settledAt: board.answer ? Date.parse(board.answer.created_at) : undefined,
      recordedBy: board.comment.recorded_by,
      board,
    })
  }
  for (const harness of asks) {
    const key = questionKey(harness.session_id, harness.question_id)
    const board = merged.get(key)
    merged.set(key, {
      ...board,
      key,
      sessionId: harness.session_id,
      questionId: harness.question_id,
      ticketId: harness.about.ticket ?? board?.ticketId,
      questions: harness.questions,
      about: harness.about,
      askedAt: harness.asked_at,
      state: harness.state,
      answerText: harness.answer_text,
      by: harness.by,
      outcome: harness.outcome,
      settledAt: harness.settled_at,
      harness,
    })
  }
  return [...merged.values()].sort((a, b) => b.askedAt - a.askedAt)
}

export function progressFor(
  subtasks: ControlPlaneTicket[],
  releases: ControlPlaneRelease[],
  doneStatus: string,
  canceledStatus?: string | null,
): ControlPlaneProgress[] {
  const groups = new Map<string | null, ControlPlaneTicket[]>()
  for (const task of subtasks) {
    if (task.status === canceledStatus) continue
    const releaseId = task.release_id ?? null
    groups.set(releaseId, [...(groups.get(releaseId) ?? []), task])
  }
  return [...groups].map(([releaseId, tasks]) => ({
    releaseId,
    releaseName: releases.find((release) => release.id === releaseId)?.name ?? 'No release',
    done: tasks.filter((task) => task.status === doneStatus).length,
    inProgress: tasks.filter((task) => task.status === 'in_progress').length,
    total: tasks.length,
  }))
}

function newestProgress(
  session: SessionMeta,
  card?: ControlPlaneTicket,
  subtasks: ControlPlaneTicket[] = [],
): number {
  if (!card && subtasks.length === 0) return session.updated_at
  return Math.max(
    card ? Date.parse(card.updated_at) : 0,
    ...subtasks.map((task) => Date.parse(task.updated_at)),
  )
}

export function cardForSession(
  session: Pick<SessionMeta, 'metadata'>,
  cards: readonly ControlPlaneTicket[],
): ControlPlaneTicket | undefined {
  const ticket = linkedTicket(session)
  return ticket
    ? cards.find(
        (card) =>
          card.id === ticket.trim() || card.key.toLowerCase() === ticket.trim().toLowerCase(),
      )
    : undefined
}

export function checkState(rows: Array<{ bucket?: string; state?: string }>): {
  checks: ControlPlanePullRequest['checks']
  failing: number
} {
  if (rows.length === 0) return { checks: 'none', failing: 0 }
  const failing = rows.filter(
    (row) => row.bucket === 'fail' || /fail|error|cancel/i.test(row.state ?? ''),
  ).length
  if (failing) return { checks: 'failing', failing }
  const running = rows.some(
    (row) => row.bucket === 'pending' || /pending|queued|progress/i.test(row.state ?? ''),
  )
  return { checks: running ? 'running' : 'passing', failing: 0 }
}

export async function fetchPullRequests(
  iii: ControlPlaneClient,
  repositories: string[],
  cards: ControlPlaneTicket[],
): Promise<ControlPlanePullRequest[]> {
  const byRepo = await Promise.all(
    repositories.map(async (repo) => {
      const response = await iii.call<{ value?: unknown }>('github::pr::list', {
        repo,
        state: 'open',
        limit: 1_000,
      })
      const rows = Array.isArray(response.value) ? response.value : []
      return Promise.all(
        rows.map(async (raw) => {
          const row = raw as Record<string, unknown>
          const number = Number(row.number)
          const [checksResponse, viewResponse] = await Promise.all([
            iii.call<{ value?: unknown }>('github::pr::checks', {
              repo,
              number,
            }),
            iii.call<{ value?: unknown }>('github::pr::view', { repo, number }),
          ])
          const checks = checkState(
            Array.isArray(checksResponse.value)
              ? (checksResponse.value as Array<{
                  bucket?: string
                  state?: string
                }>)
              : [],
          )
          const view =
            viewResponse.value && typeof viewResponse.value === 'object'
              ? (viewResponse.value as Record<string, unknown>)
              : {}
          const headRefName = String(row.headRefName ?? '')
          const card = linkedCard(String(row.title ?? ''), headRefName, cards)
          const facts = {
            isDraft: Boolean(row.isDraft),
            checks: checks.checks,
            reviewDecision: typeof view.reviewDecision === 'string' ? view.reviewDecision : null,
            mergeable: typeof view.mergeable === 'string' ? view.mergeable : null,
            mergeStateStatus:
              typeof view.mergeStateStatus === 'string' ? view.mergeStateStatus : null,
          }
          return {
            repo,
            number,
            title: String(row.title ?? ''),
            url: String(row.url ?? ''),
            headRefName,
            isDraft: Boolean(row.isDraft),
            createdAt: String(row.createdAt ?? ''),
            updatedAt: String(row.updatedAt ?? ''),
            ...checks,
            reviewDecision: facts.reviewDecision,
            mergeable: facts.mergeable,
            card,
            waitReason: prWaitReason(facts),
          } satisfies ControlPlanePullRequest
        }),
      )
    }),
  )
  return byRepo.flat().sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
}

export interface RunStatus extends HarnessStatus {
  step?: number
  context?: { total: number } | null
}

export async function fetchControlPlaneRuns(
  iii: ControlPlaneClient,
  contextCache = new Map<string, { key: string; total: number }>(),
): Promise<{ rows: RunSnapshot[]; excluded: Set<string> }> {
  const sessions: SessionMeta[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{ sessions: SessionMeta[]; next_cursor?: string }>(
      'session::list',
      {
        roots: true,
        kinds: ['user', 'automation'],
        limit: 200,
        ...(cursor ? { cursor } : {}),
      },
    )
    sessions.push(...page.sessions)
    cursor = page.next_cursor
  } while (cursor)
  const excluded = new Set(
    sessions
      .filter(
        (session) => session.metadata?.side_chat_of || session.metadata?.origin === 'control-plane',
      )
      .map((session) => session.session_id),
  )
  const roots = sessions.filter(
    (session) =>
      !excluded.has(session.session_id) && !session.metadata?.spawned_by && session.kind !== 'e2e',
  )
  const rows = await Promise.all(
    roots.map(async (session) => {
      const [status, tail] = await Promise.all([
        iii.call<RunStatus | null>('harness::status', {
          session_id: session.session_id,
          verbose: true,
        }),
        iii.call<{ messages: TranscriptItem[] }>('session::messages-tail', {
          session_id: session.session_id,
          limit: 8,
          include_custom: false,
          include_image_data: false,
        }),
      ])
      const key = `${status?.turn_id}:${status?.step}:${status?.context != null}`
      let context = contextCache.get(session.session_id)
      if (!status?.turn_id || context?.key !== key) {
        context = {
          key,
          total:
            status?.turn_id && status.context
              ? status.context.total
              : estimateTail(tail.messages ?? []),
        }
        contextCache.set(session.session_id, context)
      }
      const effective: HarnessStatus = status ?? {
        session_id: session.session_id,
        status: session.status,
        children: [],
        expects_wake: false,
        open_asks: 0,
      }
      const descendants: HarnessStatus[] = []
      const seen = new Set([session.session_id])
      const visit = async (children: HarnessStatus['children']) => {
        for (const child of children ?? []) {
          if (seen.has(child.session_id) || excluded.has(child.session_id)) continue
          seen.add(child.session_id)
          const childStatus = await iii.call<HarnessStatus | null>('harness::status', {
            session_id: child.session_id,
            verbose: true,
          })
          if (!childStatus) continue
          descendants.push(childStatus)
          await visit(childStatus.children)
        }
      }
      await visit(effective.children)
      const subtasks = linkedTicket(session)
        ? await iii
            .call<{ tickets: ControlPlaneTicket[] }>('kanban::ticket::list', {
              parent_id: linkedTicket(session),
            })
            .then((result) => result.tickets)
            .catch(() => null)
        : []
      return {
        session,
        status: effective,
        subtasks,
        runningChildren: descendants.filter((child) => child.status === 'running').length,
        childOpenAsks: descendants.reduce((sum, child) => sum + (child.open_asks ?? 0), 0),
        descendantSessionIds: descendants.map((child) => child.session_id),
        lastAssistant: lastAssistant(tail.messages ?? []),
        context_size: context?.total ?? 0,
      }
    }),
  )
  return { rows, excluded }
}

function estimateTail(items: TranscriptItem[]): number {
  return items.reduce(
    (sum, item) =>
      sum +
      Math.ceil(
        (item.message?.content ?? []).reduce(
          (chars, block) =>
            chars +
            (typeof block.text === 'string' ? block.text.length : JSON.stringify(block).length),
          0,
        ) / 4,
      ),
    0,
  )
}

const DEFAULT_ESCALATE = ['security', 'publish', 'external_cost', 'data']
const live = (run: ControlPlaneRun) => ['working', 'waiting', 'stalled'].includes(run.state)
const open = (state: string) => ['open', 'answering', 'expired'].includes(state)

export function sourceHash(
  data: Pick<ControlPlaneData, 'tickets' | 'runs' | 'prs' | 'questions' | 'completed_root_turns'>,
): string {
  const stable = (values: unknown[]) => values.map((value) => JSON.stringify(value)).sort()
  return createHash('sha256')
    .update(
      JSON.stringify({
        cards: [
          ...new Set(
            stable(
              [...data.tickets, ...data.runs.flatMap((run) => run.subtasks ?? [])].map((card) => [
                card.id,
                card.updated_at,
              ]),
            ),
          ),
        ],
        completed:
          data.completed_root_turns ??
          stable(
            data.runs
              .filter((run) => ['completed', 'failed', 'cancelled'].includes(run.status.status))
              .map((run) => [run.session.session_id, run.status.turn_id]),
          ),
        // github::pr::list/view expose the branch and updatedAt, but no head SHA.
        prs: stable(
          data.prs.map((pr) => [
            pr.repo,
            pr.number,
            pr.headRefName,
            pr.updatedAt,
            pr.checks,
            pr.failing,
          ]),
        ),
        asks: stable(
          data.questions
            .filter((question) => open(question.state))
            .map((question) => [question.key, question.state]),
        ),
      }),
    )
    .digest('hex')
}

export function projectSnapshot(
  board: BoardSnapshot,
  rows: RunSnapshot[],
  asks: AskEntry[],
  boardQuestions: KanbanQuestionView[],
  prs: ControlPlaneData['prs'],
  scope: string,
  config: ControlPlaneConfig,
  now = Date.now(),
): ControlPlaneData {
  const cards = board.tickets
  const findCard = (ticket?: string) =>
    cards.find(
      (card) =>
        ticket &&
        (card.id === ticket.trim() || card.key.toLowerCase() === ticket.trim().toLowerCase()),
    )
  const rootFor = (id: string) =>
    rows.find((row) => row.session.session_id === id || row.descendantSessionIds.includes(id))
  const questions = mergeControlPlaneQuestions(asks, boardQuestions).filter((question) => {
    const card = findCard(question.ticketId ?? linkedTicket(rootFor(question.sessionId)?.session))
    return scope === ALL_PROJECTS || Boolean(question.board) || isInScope(scope, card?.project_id)
  })
  const escalates = (projectId?: string | null) =>
    projectId ? (config.escalate[projectId] ?? DEFAULT_ESCALATE) : DEFAULT_ESCALATE
  const basic: ControlPlaneData['items'] = questions.flatMap((record) => {
    if (!open(record.state)) return []
    const row = rootFor(record.sessionId)
    const ask: AskEntry = record.harness ?? {
      session_id: record.sessionId,
      question_id: record.questionId,
      turn_id: '',
      questions: record.questions,
      about: record.about,
      asked_at: record.askedAt,
      on_expiry: 'block',
      state: record.state as 'open',
    }
    const card = findCard(ask.about.ticket ?? record.ticketId ?? linkedTicket(row?.session))
    const blocks = ask.about.blocks ?? []
    return [
      {
        id: `question:${ask.session_id}:${ask.question_id}`,
        kind: 'question' as const,
        sessionId: ask.session_id,
        questionId: ask.question_id,
        title: ask.questions[0]?.question ?? 'Question',
        question: ask.questions[0]?.question ?? '',
        questionClass: ask.about.class ?? 'scope',
        why: ask.about.why,
        blocks,
        blockedCards: blocks.length,
        createdAt: ask.asked_at,
        state: ask.state as 'open',
        entry: ask,
        sessionTitle: row?.session.title,
        currentTurnId: row?.status.turn_id,
        card,
        urgency: escalates(card?.project_id).includes(ask.about.class ?? 'scope')
          ? ('now' as const)
          : ('no-rush' as const),
      },
    ]
  })
  const runs: ControlPlaneData['runs'] = rows.flatMap((row) => {
    const card = cardForSession(row.session, cards)
    if (!isInScope(scope, card?.project_id)) return []
    const lastProgress = newestProgress(row.session, card, row.subtasks ?? [])
    const openQuestions = (row.status.open_asks ?? 0) + row.childOpenAsks
    const stalled =
      row.subtasks !== null &&
      isStalled(
        {
          status: row.session.status,
          runningChildren: row.runningChildren,
          openQuestions,
          expectsWake: row.status.expects_wake,
          armedWakes: row.status.armed_wakes,
          lastProgress,
        },
        now,
        config.stall_after_min,
      )
    const active =
      row.session.status === 'working' ||
      ['running', 'awaiting_functions'].includes(row.status.status) ||
      row.runningChildren > 0
    const linkedRecently = Boolean(card) && now - row.session.updated_at < 24 * 60 * 60_000
    if (!active && !openQuestions && !row.status.expects_wake && !linkedRecently) return []
    const profile = row.session.metadata?.agent_profile
    return [
      {
        session: {
          session_id: row.session.session_id,
          title: row.session.title,
          created_at: row.session.created_at,
          updated_at: row.session.updated_at,
        },
        status: {
          session_id: row.status.session_id,
          turn_id: row.status.turn_id,
          status: row.status.status,
          children: row.status.children.map(({ session_id }) => ({ session_id })),
          expects_wake: row.status.expects_wake,
          open_asks: row.status.open_asks,
          armed_wakes: row.status.armed_wakes?.map(({ created_at, expires_at }) => ({
            created_at,
            expires_at,
          })),
        },
        card,
        subtasks: row.subtasks,
        progress: progressFor(
          row.subtasks ?? [],
          board.releaseCatalog,
          board.board.doneStatus,
          board.board.canceledStatus,
        ),
        needsYou: [],
        lastProgress,
        lastAssistant: row.lastAssistant,
        context_size: row.context_size,
        state: stalled
          ? ('stalled' as const)
          : row.session.status === 'error'
            ? ('failed' as const)
            : active
              ? ('working' as const)
              : openQuestions || row.status.expects_wake
                ? ('waiting' as const)
                : ('done' as const),
        profile:
          typeof profile === 'string'
            ? profile
            : profile && typeof profile === 'object' && 'id' in profile
              ? String(profile.id)
              : undefined,
        startedBy: row.session.session_id.startsWith('dispatch-')
          ? ('board dispatch' as const)
          : row.session.kind === 'automation'
            ? ('automation' as const)
            : ('chat' as const),
        descendantSessionIds: row.descendantSessionIds,
      },
    ]
  })
  const items: ControlPlaneData['items'] = [
    ...basic,
    ...runs
      .filter((run) => run.state === 'stalled')
      .map((run) => ({
        id: `stalled:${run.session.session_id}`,
        kind: 'stalled' as const,
        sessionId: run.session.session_id,
        title: run.session.title,
        blockedCards: run.card ? 1 : 0,
        createdAt: run.lastProgress,
        lastProgress: run.lastProgress,
        armedWakes: run.status.armed_wakes,
        card: run.card,
        urgency: 'now' as const,
      })),
  ].sort(
    (a, b) =>
      Number(b.urgency === 'now') - Number(a.urgency === 'now') ||
      b.blockedCards - a.blockedCards ||
      a.createdAt - b.createdAt,
  )
  for (const run of runs)
    run.needsYou = items.filter(
      (item) =>
        item.sessionId === run.session.session_id ||
        run.descendantSessionIds.includes(item.sessionId) ||
        Boolean(item.card && item.card.id === run.card?.id),
    )
  const direction: Direction[] = board.releaseCatalog
    .filter((release) => release.target_at)
    .map((release) => {
      const target = Date.parse(release.target_at ?? '')
      const reasons: Direction['reasons'] = []
      if (target < now) reasons.push('past-target')
      if (
        target >= now &&
        target <= now + config.at_risk_days * 86400000 &&
        release.done < release.total
      )
        reasons.push('open-cards')
      if (
        asks.some((ask) => {
          const card = findCard(ask.about.ticket ?? linkedTicket(rootFor(ask.session_id)?.session))
          return (
            open(ask.state) &&
            card?.release_id === release.id &&
            escalates(card.project_id).includes(ask.about.class ?? 'scope')
          )
        })
      )
        reasons.push('escalated-question')
      return {
        release,
        state: reasons.length ? ('at-risk' as const) : ('on-track' as const),
        reasons,
      }
    })
  const waits = questions
    .flatMap((question) => (question.harness ? [question.harness] : []))
    .filter(
      (ask) =>
        !open(ask.state) &&
        ask.settled_at != null &&
        ask.settled_at >= now - 7 * 86400000 &&
        ask.settled_at <= now &&
        ask.settled_at >= ask.asked_at,
    )
    .map((ask) => (ask.settled_at ?? 0) - ask.asked_at)
    .sort((a, b) => a - b)
  const selected = board.projects.filter((project) => isInScope(scope, project.id))
  const result: ControlPlaneData = {
    ...board,
    completed_root_turns: rows
      .filter(
        (row) =>
          isInScope(scope, cardForSession(row.session, cards)?.project_id) &&
          (row.completedTurnId || ['completed', 'failed', 'cancelled'].includes(row.status.status)),
      )
      .map((row) => `${row.session.session_id}/${row.completedTurnId ?? row.status.turn_id ?? ''}`)
      .sort(),
    as_of: new Date(now).toISOString(),
    sourceHash: '',
    runs,
    items,
    questions,
    prs,
    repositories: [...new Set(selected.flatMap((project) => project.repos ?? []))].sort(),
    direction,
    exceptions: {
      stalled: runs.filter((run) => run.state === 'stalled'),
      failing_prs: prs.filter((pr) => pr.checks === 'failing'),
      blocked_cards: cards.filter((card) => card.labels.includes('blocked')),
      old_questions: questions.filter(
        (question) => open(question.state) && now - question.askedAt > 86400000,
      ),
    },
    median_decision_wait_ms: waits.length
      ? (waits[Math.floor((waits.length - 1) / 2)] + waits[Math.floor(waits.length / 2)]) / 2
      : null,
    loading: { board: false, sessions: false, needsYou: false, questions: false, prs: false },
    errors: { board: null, sessions: null, needsYou: null, questions: null, prs: null },
  }
  result.sourceHash = sourceHash(result)
  return result
}

interface ScopeSources {
  board: BoardSnapshot
  rows: RunSnapshot[]
  asks: AskEntry[]
  questions: KanbanQuestionView[]
  excluded: Set<string>
  errors: ControlPlaneData['errors']
}
interface ScopeCache {
  sources: ScopeSources
  data: ControlPlaneData
}
interface RepositoryCache {
  at: number
  prs: ControlPlaneData['prs']
  error: string | null
  pending?: Promise<void>
}
const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error))

export class ControlPlaneReadModel {
  private scopes = new Map<string, ScopeCache>()
  private watched = new Map<string, number>()
  private repositories = new Map<string, RepositoryCache>()
  private contexts = new Map<string, { key: string; total: number }>()
  private pending = new Map<string, Promise<ControlPlaneData>>()
  private dirty = new Set<string>()
  private debounce?: ReturnType<typeof setTimeout>
  private deadline?: ReturnType<typeof setTimeout>
  private stopped = false

  constructor(
    private iii: ControlPlaneClient,
    private config: () => ControlPlaneConfig,
    private changed: (scope: string, as_of: string, parts: string[]) => Promise<void> | void,
  ) {}

  activeScopes(): string[] {
    const now = Date.now()
    return [...new Set([...this.watched.keys(), ...this.scopes.keys()])].filter(
      (scope) =>
        (this.watched.get(scope) ?? 0) > now ||
        this.scopes.get(scope)?.data.runs.some((run) => live(run)),
    )
  }

  knownScopes(): string[] {
    return [...this.scopes.keys()]
  }

  invalidatePrs(): void {
    this.repositories.clear()
  }

  async boot(): Promise<void> {
    await this.rebuild('all')
    await this.materializeLiveProjects()
  }

  private async materializeLiveProjects(): Promise<void> {
    const projects = new Set(
      this.scopes
        .get('all')
        ?.data.runs.filter(live)
        .map((run) => run.card?.project_id)
        .filter((id): id is string => Boolean(id)),
    )
    for (const scope of projects) await this.read(scope)
  }

  async read(scope: string): Promise<ControlPlaneData> {
    return this.scopes.get(scope)?.data ?? this.pending.get(scope) ?? this.rebuild(scope)
  }

  async watch(scope: string): Promise<{ watched_until: string }> {
    const now = Date.now()
    const firstWatch = (this.watched.get(scope) ?? 0) <= now
    const until = now + 120_000
    this.watched.set(scope, until)
    await this.update(scope, firstWatch)
    return { watched_until: new Date(until).toISOString() }
  }

  onSource(event: unknown, turn = false): void {
    if (turn && !isRootTurnEvent(event)) return
    const id =
      event && typeof event === 'object' && 'session_id' in event ? String(event.session_id) : ''
    if ([...this.scopes.values()].some((cache) => cache.sources.excluded.has(id))) return
    if (this.debounce) clearTimeout(this.debounce)
    this.debounce = setTimeout(() => {
      void this.rebuildScopes().catch((error) => console.error('control-plane rebuild:', error))
    }, 2000)
  }

  private async rebuildScopes(): Promise<void> {
    for (const scope of new Set([...this.scopes.keys(), ...this.pending.keys()]))
      await this.rebuild(scope)
    await this.materializeLiveProjects()
  }

  rebuild(scope: string): Promise<ControlPlaneData> {
    return this.update(scope, true)
  }

  private update(scope: string, refreshSources: boolean): Promise<ControlPlaneData> {
    const pending = this.pending.get(scope)
    if (pending) {
      if (refreshSources) this.dirty.add(scope)
      return pending
    }
    const work = (async () => {
      try {
        let data = await this.build(scope, refreshSources)
        while (this.dirty.delete(scope)) data = await this.build(scope, true)
        return data
      } finally {
        this.pending.delete(scope)
      }
    })()
    this.pending.set(scope, work)
    return work
  }

  private async build(scope: string, refreshSources: boolean): Promise<ControlPlaneData> {
    const old = this.scopes.get(scope)
    const sources =
      refreshSources || !old ? await this.fetchScope(scope, old?.sources) : old.sources
    const { board, rows, asks, questions, errors } = sources
    const data = projectSnapshot(board, rows, asks, questions, [], scope, this.config())
    data.errors = { ...errors }
    const cache = { sources, data }
    await this.updatePrs(scope, cache)
    this.scopes.set(scope, cache)
    this.armDeadline()
    await this.publish(scope, old?.data, data).catch((error) =>
      console.error('control-plane change notification:', error),
    )
    return data
  }

  private async fetchScope(scope: string, old?: ScopeSources): Promise<ScopeSources> {
    const sources = await Promise.allSettled([
      fetchControlPlaneBoard(this.iii, scope),
      fetchControlPlaneRuns(this.iii, this.contexts),
      fetchControlPlaneAsks(this.iii),
      fetchControlPlaneQuestions(this.iii, scope),
    ] as const)
    const emptyBoard: BoardSnapshot = {
      projects: [],
      releases: [],
      releaseCatalog: [],
      tickets: [],
      board: { columns: [], doneStatus: 'done', idPrefix: 'KAN' },
    }
    const board = sources[0].status === 'fulfilled' ? sources[0].value : (old?.board ?? emptyBoard)
    const completed = new Map(old?.rows.map((row) => [row.session.session_id, row.completedTurnId]))
    const rows =
      sources[1].status === 'fulfilled'
        ? sources[1].value.rows.map((row) => ({
            ...row,
            completedTurnId: ['completed', 'failed', 'cancelled'].includes(row.status.status)
              ? (row.status.turn_id ?? undefined)
              : completed.get(row.session.session_id),
          }))
        : (old?.rows ?? [])
    const excluded =
      sources[1].status === 'fulfilled'
        ? sources[1].value.excluded
        : (old?.excluded ?? new Set<string>())
    const known = new Set(
      rows.flatMap((row) => [row.session.session_id, ...row.descendantSessionIds]),
    )
    const allowed = (id: string) =>
      !excluded.has(id) && (sources[1].status === 'fulfilled' || known.has(id))
    const asks = (sources[2].status === 'fulfilled' ? sources[2].value : (old?.asks ?? [])).filter(
      (ask) => allowed(ask.session_id),
    )
    const questions = (
      sources[3].status === 'fulfilled' ? sources[3].value : (old?.questions ?? [])
    ).filter(
      (question) => !question.comment.question || allowed(question.comment.question.session_id),
    )
    const errors: ControlPlaneData['errors'] = {
      board: null,
      sessions: null,
      needsYou: null,
      questions: null,
      prs: null,
    }
    const names = ['board', 'sessions', 'needsYou', 'questions'] as const
    for (const [index, result] of sources.entries())
      if (result.status === 'rejected') errors[names[index]] = messageOf(result.reason)
    errors.needsYou ??= errors.board ?? errors.sessions ?? errors.questions
    return { board, rows, asks, questions, excluded, errors }
  }

  private repoInterval(repo: string, scopes: Map<string, ScopeCache>): number | null {
    let hasLive = false
    const config = this.config()
    for (const [scope, cache] of scopes) {
      if (!cache.data.repositories.includes(repo)) continue
      if ((this.watched.get(scope) ?? 0) > Date.now())
        return config.prs.watched_interval_min * 60000
      const projects = cache.sources.board.projects
        .filter((project) => project.repos?.includes(repo))
        .map((project) => project.id)
      hasLive ||= cache.data.runs.some(
        (run) => projects.includes(run.card?.project_id ?? '') && live(run),
      )
    }
    return hasLive ? config.prs.idle_interval_min * 60000 : null
  }

  private async updatePrs(scope: string, cache: ScopeCache): Promise<void> {
    const { data } = cache
    const scopes = new Map(this.scopes).set(scope, cache)
    await Promise.all(
      data.repositories.map(async (repo) => {
        let cached = this.repositories.get(repo)
        const interval = this.repoInterval(repo, scopes)
        if (cached?.pending) await cached.pending
        else if (interval !== null && (!cached || Date.now() - cached.at >= interval)) {
          cached ??= { at: 0, prs: [], error: null }
          this.repositories.set(repo, cached)
          const target = cached
          target.pending = (async () => {
            try {
              const prs = await fetchPullRequests(this.iii, [repo], [])
              const at = Date.now()
              target.prs = prs.map((pr) => ({ ...pr, as_of: new Date(at).toISOString() }))
              target.at = at
              target.error = null
            } catch (error) {
              target.error = messageOf(error)
              target.at = Date.now()
            }
          })().finally(() => {
            target.pending = undefined
          })
          await target.pending
        }
      }),
    )
    const cached = data.repositories.map((repo) => this.repositories.get(repo))
    data.errors.prs = cached.find((cache) => cache?.error)?.error ?? null
    data.prs = cached
      .flatMap((cache) => (cache?.error ? [] : (cache?.prs ?? [])))
      .map((pr) => ({ ...pr, card: linkedCard(pr.title, pr.headRefName, data.tickets) }))
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    data.exceptions.failing_prs = data.prs.filter((pr) => pr.checks === 'failing')
    data.sourceHash = sourceHash(data)
  }

  private async publish(
    scope: string,
    old: ControlPlaneData | undefined,
    next: ControlPlaneData,
  ): Promise<void> {
    const parts = [
      'projects',
      'tickets',
      'releaseCatalog',
      'repositories',
      'completed_root_turns',
      'direction',
      'runs',
      'items',
      'questions',
      'prs',
      'board',
      'releases',
      'exceptions',
      'errors',
      'median_decision_wait_ms',
    ].filter(
      (part) =>
        JSON.stringify(old?.[part as keyof ControlPlaneData]) !==
        JSON.stringify(next[part as keyof ControlPlaneData]),
    )
    if (parts.length) await this.changed(scope, next.as_of, parts)
  }

  private armDeadline(): void {
    if (this.deadline) clearTimeout(this.deadline)
    if (this.stopped || !this.scopes.size) return
    const now = Date.now()
    const boundaries = [now + 60000]
    for (const cache of this.scopes.values())
      for (const run of cache.data.runs) {
        boundaries.push(
          run.lastProgress + this.config().stall_after_min * 60000,
          run.session.updated_at + 86400000,
        )
        for (const wake of run.status.armed_wakes ?? [])
          if (wake.expires_at) boundaries.push(wake.expires_at)
      }
    const next = Math.min(...boundaries.filter((at) => at > now))
    this.deadline = setTimeout(() => {
      void this.tick().catch((error) => console.error('control-plane deadline:', error))
    }, next - now)
    this.deadline.unref?.()
  }

  async tick(): Promise<void> {
    const active = new Set(this.activeScopes())
    for (const scope of this.scopes.keys()) await this.update(scope, active.has(scope))
  }

  dispose(): void {
    this.stopped = true
    if (this.debounce) clearTimeout(this.debounce)
    if (this.deadline) clearTimeout(this.deadline)
  }
}
