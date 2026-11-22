import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { readGitSummary } from '@/lib/chat-aside'
import type { IiiClient, ObserveSource } from '@/lib/iii-client'
import type {
  AgentMessage,
  SessionMeta,
  TranscriptItem,
} from '@/lib/sessions/types'
import {
  ALL_PROJECTS,
  type AskEntry,
  type HarnessStatus,
  isInScope,
  isStalled,
  type KanbanQuestionView,
  linkedCard,
  type NeedsYouItem,
  prWaitReason,
  sortNeeds,
  type TowerBoard,
  type TowerProgress,
  type TowerProject,
  type TowerPullRequest,
  type TowerQuestion,
  type TowerRelease,
  type TowerRun,
  type TowerTicket,
} from './model'

interface PendingApproval {
  session_id: string
  turn_id: string
  function_call_id: string
  function_id: string
  namespace?: string
  implicit_namespace?: string
  arguments_excerpt: unknown
  pending_at: number
  session_title?: string
  session_metadata?: Record<string, unknown>
}

interface BoardSnapshot {
  projects: TowerProject[]
  releases: TowerRelease[]
  board: TowerBoard
  tickets: TowerTicket[]
  releaseCatalog: TowerRelease[]
}

interface RunSnapshot {
  session: SessionMeta
  status: HarnessStatus
  subtasks: TowerTicket[] | null
  runningChildren: number
  childOpenAsks: number
  descendantSessionIds: string[]
  descendants: Array<{ meta?: SessionMeta; status: HarnessStatus }>
  lastAssistant: string
  metrics?: TowerRun['metrics']
}

interface LiveResult<T> {
  data: T | null
  loading: boolean
  error: string | null
  refresh: () => void
}

export interface TowerData {
  projects: TowerProject[]
  board: TowerBoard | null
  tickets: TowerTicket[]
  releases: TowerRelease[]
  releaseCatalog: TowerRelease[]
  runs: TowerRun[]
  items: NeedsYouItem[]
  questions: TowerQuestion[]
  prs: TowerPullRequest[]
  repositories: string[]
  loading: {
    board: boolean
    sessions: boolean
    needsYou: boolean
    questions: boolean
    prs: boolean
  }
  errors: {
    board: string | null
    sessions: string | null
    needsYou: string | null
    questions: string | null
    prs: string | null
  }
  refresh: () => void
}

const BOARD_EVENTS: ObserveSource[] = [{ function_id: 'kanban::on-change' }]
const ASK_EVENTS: ObserveSource[] = [
  { function_id: 'harness::on-ask' },
  { function_id: 'harness::on-ask-settled' },
]
const HOLD_EVENTS: ObserveSource[] = [
  { function_id: 'approval::on-pending-created' },
  { function_id: 'approval::on-pending-resolved' },
]
const SESSION_EVENTS: ObserveSource[] = [
  { function_id: 'harness::on-turn-started' },
  { function_id: 'harness::on-turn-completed' },
  // Run progress is a card-subtask read, so board changes refresh it too.
  { function_id: 'kanban::on-change' },
]
const ACCEPT_ALL = () => true

export function isRootTurnEvent(payload: unknown): boolean {
  if (!payload || typeof payload !== 'object') return false
  const event = payload as Record<string, unknown>
  if (typeof event.depth === 'number') return event.depth === 0
  return event.parent == null && event.parent_session_id == null
}

function useLiveSource<T>(
  iii: IiiClient,
  handlerId: string,
  sources: readonly ObserveSource[],
  fetch: () => Promise<T>,
  acceptEvent: (payload: unknown) => boolean = ACCEPT_ALL,
): LiveResult<T> {
  const [snapshot, setSnapshot] = useState<{
    source: () => Promise<T>
    data: T | null
  }>({ source: fetch, data: null })
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)
  const instance = useId().replace(/[^a-z0-9]/gi, '')
  const registeredHandlerId = `${handlerId}-${instance}`
  const seq = useRef(0)
  const debounce = useRef<ReturnType<typeof setTimeout> | null>(null)
  const refresh = useCallback(() => setToken((value) => value + 1), [])
  const sourceKey = sources.map((source) => JSON.stringify(source)).join('\0')

  // biome-ignore lint/correctness/useExhaustiveDependencies: token deliberately re-runs the fetch
  useEffect(() => {
    const mine = ++seq.current
    setLoading(true)
    void fetch().then(
      (next) => {
        if (mine !== seq.current) return
        setSnapshot({ source: fetch, data: next })
        setError(null)
        setLoading(false)
      },
      (reason) => {
        if (mine !== seq.current) return
        setSnapshot((previous) =>
          previous.source === fetch ? previous : { source: fetch, data: null },
        )
        setError(errorMessage(reason))
        setLoading(false)
      },
    )
  }, [fetch, token])

  // biome-ignore lint/correctness/useExhaustiveDependencies: sourceKey represents the source array contents
  useEffect(() => {
    const schedule = (payload: unknown) => {
      if (!acceptEvent(payload)) return
      if (debounce.current) clearTimeout(debounce.current)
      debounce.current = setTimeout(refresh, 2_000)
    }
    const offs: Array<() => void> = []
    let subscribed = true
    try {
      offs.push(iii.on(registeredHandlerId, schedule))
      const target = `${registeredHandlerId}::${iii.browserId}`
      for (const source of sources) offs.push(iii.observe(source, target))
    } catch {
      subscribed = false
    }
    const onFocus = () => {
      if (!document.hidden) refresh()
    }
    if (!subscribed || sources.length === 0) {
      window.addEventListener('focus', onFocus)
      document.addEventListener('visibilitychange', onFocus)
    }
    return () => {
      if (debounce.current) clearTimeout(debounce.current)
      window.removeEventListener('focus', onFocus)
      document.removeEventListener('visibilitychange', onFocus)
      for (const off of offs) off()
    }
  }, [acceptEvent, iii, refresh, registeredHandlerId, sourceKey])

  const current = snapshot.source === fetch
  return {
    data: current ? snapshot.data : null,
    loading: loading || !current,
    error: current ? error : null,
    refresh,
  }
}

function fsRoot(session: SessionMeta): string | undefined {
  const scope = session.metadata?.fs_scope
  if (!scope || typeof scope !== 'object') return undefined
  const root = (scope as Record<string, unknown>).root
  return typeof root === 'string' && root ? root : undefined
}

function linkedTicket(session?: SessionMeta): string | undefined {
  const ticket = session?.metadata?.kanban_ticket
  return typeof ticket === 'string' && ticket ? ticket : undefined
}

function lastAssistant(items: TranscriptItem[]): string {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const message: AgentMessage | undefined = items[index].message
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

export async function fetchTowerBoard(
  iii: IiiClient,
  scope: string,
): Promise<BoardSnapshot> {
  const [projectsResult, releasesResult, boardResult] = await Promise.all([
    iii.call<{ projects: TowerProject[] }>('kanban::project::list', {}),
    iii.call<{ releases: TowerRelease[] }>('kanban::release::list', {}),
    iii.call<{
      columns: TowerBoard['columns']
      done_status: string
      canceled_status?: string | null
      id_prefix: string
    }>(
      'kanban::board::get',
      scope === ALL_PROJECTS ? {} : { project_id: scope },
    ),
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
    .sort((a, b) =>
      (a.target_at ?? '9999').localeCompare(b.target_at ?? '9999'),
    )
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

async function stallAfterMin(iii: IiiClient): Promise<number> {
  try {
    const response = await iii.call<{ value?: unknown }>('configuration::get', {
      id: await resolveConfigurationId(iii, 'ade'),
      raw: true,
    })
    const root = response.value
    const tower =
      root && typeof root === 'object'
        ? (root as Record<string, unknown>).tower
        : undefined
    const value =
      tower && typeof tower === 'object'
        ? (tower as Record<string, unknown>).stall_after_min
        : undefined
    return typeof value === 'number' && value > 0 ? value : 30
  } catch {
    return 30
  }
}

async function sessionDetails(
  iii: IiiClient,
  listed: SessionMeta,
): Promise<RunSnapshot | null> {
  const [get, status, tail, metrics] = await Promise.all([
    iii.call<{ meta: SessionMeta } | null>('session::get', {
      session_id: listed.session_id,
    }),
    iii.call<HarnessStatus | null>('harness::status', {
      session_id: listed.session_id,
      verbose: true,
    }),
    iii.call<{ messages: TranscriptItem[] }>('session::messages-tail', {
      session_id: listed.session_id,
      limit: 8,
      include_custom: false,
      include_image_data: false,
    }),
    iii
      .call<TowerRun['metrics']>('harness::metrics', {
        root_session_id: listed.session_id,
        subtree: true,
        omit_context: true,
        omit_traces: true,
      })
      .catch(() => undefined),
  ])
  const session = get?.meta ?? listed
  if (!status) return null
  const descendants = async (
    children: HarnessStatus['children'],
  ): Promise<Array<{ meta?: SessionMeta; status: HarnessStatus }>> => {
    const direct = await Promise.all(
      children.map(async (child) => {
        const [meta, childStatus] = await Promise.all([
          iii.call<{ meta: SessionMeta } | null>('session::get', {
            session_id: child.session_id,
          }),
          iii.call<HarnessStatus | null>('harness::status', {
            session_id: child.session_id,
            verbose: true,
          }),
        ])
        return childStatus ? { meta: meta?.meta, status: childStatus } : null
      }),
    )
    const present = direct.filter(
      (
        child,
      ): child is { meta: SessionMeta | undefined; status: HarnessStatus } =>
        child !== null,
    )
    return [
      ...present,
      ...(
        await Promise.all(
          present.map((child) => descendants(child.status.children ?? [])),
        )
      ).flat(),
    ]
  }
  const children = await descendants(status.children ?? [])
  const ticket = linkedTicket(session)
  const subtasks = ticket
    ? await iii
        .call<{ tickets: TowerTicket[] }>('kanban::ticket::list', {
          parent_id: ticket,
        })
        .then((result) => result.tickets)
        .catch(() => null)
    : []
  return {
    session,
    status,
    subtasks,
    runningChildren: children.filter(
      (child) =>
        child.meta?.status === 'working' || child.status.status === 'running',
    ).length,
    childOpenAsks: children.reduce(
      (total, child) => total + child.status.open_asks,
      0,
    ),
    descendantSessionIds: children.map((child) => child.status.session_id),
    descendants: children,
    lastAssistant: lastAssistant(tail.messages ?? []),
    metrics,
  }
}

export async function fetchTowerRuns(
  iii: IiiClient,
): Promise<{ rows: RunSnapshot[]; stallAfterMin: number }> {
  const [listed, stallMinutes] = await Promise.all([
    iii.call<{ sessions: SessionMeta[] }>('session::list', {
      roots: true,
      kinds: ['user', 'automation'],
      limit: 200,
    }),
    stallAfterMin(iii),
  ])
  const roots = (listed.sessions ?? []).filter(
    (session) =>
      !session.metadata?.side_chat_of && session.metadata?.origin !== 'tower',
  )
  return {
    rows: (
      await Promise.all(roots.map((session) => sessionDetails(iii, session)))
    ).filter((row): row is RunSnapshot => row !== null),
    stallAfterMin: stallMinutes,
  }
}

export async function fetchTowerAsks(
  iii: Pick<IiiClient, 'call'>,
  sessionId?: string,
): Promise<AskEntry[]> {
  const entries: AskEntry[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{ entries: AskEntry[]; cursor?: string }>(
      'harness::ask::list',
      {
        limit: 200,
        ...(sessionId ? { session_id: sessionId } : {}),
        ...(cursor ? { cursor } : {}),
      },
    )
    entries.push(...(page.entries ?? []))
    cursor = page.cursor
  } while (cursor)
  return entries
}

export async function fetchTowerQuestions(
  iii: IiiClient,
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
export function mergeTowerQuestions(
  asks: readonly AskEntry[],
  boardRows: readonly KanbanQuestionView[],
): TowerQuestion[] {
  const merged = new Map<string, TowerQuestion>()
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

export async function fetchTowerHolds(
  iii: IiiClient,
): Promise<PendingApproval[]> {
  const pending: PendingApproval[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{
      pending: PendingApproval[]
      next_cursor?: string
    }>('approval::list-pending', {
      limit: 200,
      ...(cursor ? { cursor } : {}),
    })
    pending.push(...(page.pending ?? []))
    cursor = page.next_cursor
  } while (cursor)
  return pending
}

export function progressFor(
  subtasks: TowerTicket[],
  releases: TowerRelease[],
  doneStatus: string,
  canceledStatus?: string | null,
): TowerProgress[] {
  const groups = new Map<string | null, TowerTicket[]>()
  for (const task of subtasks) {
    if (task.status === canceledStatus) continue
    const releaseId = task.release_id ?? null
    groups.set(releaseId, [...(groups.get(releaseId) ?? []), task])
  }
  return [...groups].map(([releaseId, tasks]) => ({
    releaseId,
    releaseName:
      releases.find((release) => release.id === releaseId)?.name ??
      'No release',
    done: tasks.filter((task) => task.status === doneStatus).length,
    inProgress: tasks.filter((task) => task.status === 'in_progress').length,
    total: tasks.length,
  }))
}

function newestProgress(
  session: SessionMeta,
  card?: TowerTicket,
  subtasks: TowerTicket[] = [],
): number {
  if (!card && subtasks.length === 0) return session.updated_at
  return Math.max(
    card ? Date.parse(card.updated_at) : 0,
    ...subtasks.map((task) => Date.parse(task.updated_at)),
  )
}

export function cardForSession(
  session: SessionMeta,
  cards: readonly TowerTicket[],
): TowerTicket | undefined {
  const ticket = linkedTicket(session)
  return ticket
    ? cards.find(
        (card) =>
          card.id === ticket.trim() ||
          card.key.toLowerCase() === ticket.trim().toLowerCase(),
      )
    : undefined
}

async function repositoriesFor(
  iii: IiiClient,
  scope: string,
  projects: TowerProject[],
  roots: Array<{ workingDir?: string; projectId?: string | null }>,
): Promise<string[]> {
  const selected =
    scope === ALL_PROJECTS
      ? projects
      : projects.filter((project) => project.id === scope)
  const repositories = new Set(
    selected.flatMap((project) => project.repos ?? []),
  )
  const configured = new Set(
    selected
      .filter((project) => project.repos?.length)
      .map((project) => project.id),
  )
  const folders = [
    ...new Set(
      roots
        .filter((root) => !root.projectId || !configured.has(root.projectId))
        .map((root) => root.workingDir)
        .filter((folder): folder is string => Boolean(folder)),
    ),
  ]
  const summaries = await Promise.all(
    folders.map((folder) => readGitSummary(iii.call, folder)),
  )
  for (const summary of summaries)
    if (summary?.repo) repositories.add(summary.repo)
  return [...repositories].sort()
}

export function checkState(rows: Array<{ bucket?: string; state?: string }>): {
  checks: TowerPullRequest['checks']
  failing: number
} {
  if (rows.length === 0) return { checks: 'none', failing: 0 }
  const failing = rows.filter(
    (row) =>
      row.bucket === 'fail' || /fail|error|cancel/i.test(row.state ?? ''),
  ).length
  if (failing) return { checks: 'failing', failing }
  const running = rows.some(
    (row) =>
      row.bucket === 'pending' ||
      /pending|queued|progress/i.test(row.state ?? ''),
  )
  return { checks: running ? 'running' : 'passing', failing: 0 }
}

export async function fetchPullRequests(
  iii: IiiClient,
  repositories: string[],
  cards: TowerTicket[],
): Promise<TowerPullRequest[]> {
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
            reviewDecision:
              typeof view.reviewDecision === 'string'
                ? view.reviewDecision
                : null,
            mergeable:
              typeof view.mergeable === 'string' ? view.mergeable : null,
            mergeStateStatus:
              typeof view.mergeStateStatus === 'string'
                ? view.mergeStateStatus
                : null,
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
          } satisfies TowerPullRequest
        }),
      )
    }),
  )
  return byRepo.flat().sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
}

export function useTowerData(iii: IiiClient, scope: string): TowerData {
  const fetchBoard = useCallback(
    () => fetchTowerBoard(iii, scope),
    [iii, scope],
  )
  const fetchRuns = useCallback(() => fetchTowerRuns(iii), [iii])
  const fetchAsks = useCallback(() => fetchTowerAsks(iii), [iii])
  const fetchHolds = useCallback(() => fetchTowerHolds(iii), [iii])
  const fetchQuestions = useCallback(
    () => fetchTowerQuestions(iii, scope),
    [iii, scope],
  )
  const boardLive = useLiveSource(
    iii,
    'iii::console::tower::board',
    BOARD_EVENTS,
    fetchBoard,
  )
  const sessionsLive = useLiveSource(
    iii,
    'iii::console::tower::sessions',
    SESSION_EVENTS,
    fetchRuns,
    isRootTurnEvent,
  )
  const asksLive = useLiveSource(
    iii,
    'iii::console::tower::asks',
    ASK_EVENTS,
    fetchAsks,
  )
  const holdsLive = useLiveSource(
    iii,
    'iii::console::tower::holds',
    HOLD_EVENTS,
    fetchHolds,
  )
  const questionsLive = useLiveSource(
    iii,
    'iii::console::tower::questions',
    BOARD_EVENTS,
    fetchQuestions,
  )
  const cards = boardLive.data?.tickets ?? []
  const projects = boardLive.data?.projects ?? []
  const releases = boardLive.data?.releases ?? []
  const releaseCatalog = boardLive.data?.releaseCatalog ?? []
  const [clock, setClock] = useState(Date.now)
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 60_000)
    return () => window.clearInterval(timer)
  }, [])
  const cardMap = useMemo(
    () =>
      new Map(
        cards.flatMap((card) => [
          [card.id.toLowerCase(), card],
          [card.key.toLowerCase(), card],
        ]),
      ),
    [cards],
  )
  const sessionMap = useMemo(
    () =>
      new Map(
        (sessionsLive.data?.rows ?? []).flatMap((row) => [
          [row.session.session_id, row.session] as const,
          ...row.descendants.flatMap((child) =>
            child.meta
              ? ([[child.status.session_id, child.meta]] as const)
              : [],
          ),
        ]),
      ),
    [sessionsLive.data],
  )
  const statusMap = useMemo(
    () =>
      new Map(
        (sessionsLive.data?.rows ?? []).flatMap((row) => [
          [row.status.session_id, row.status] as const,
          ...row.descendants.map(
            (child) => [child.status.session_id, child.status] as const,
          ),
        ]),
      ),
    [sessionsLive.data],
  )
  const rootForSession = useMemo(() => {
    const roots = new Map<string, RunSnapshot>()
    for (const row of sessionsLive.data?.rows ?? []) {
      roots.set(row.session.session_id, row)
      for (const child of row.descendantSessionIds) roots.set(child, row)
    }
    return roots
  }, [sessionsLive.data])

  const questions = useMemo(
    () =>
      mergeTowerQuestions(asksLive.data ?? [], questionsLive.data ?? []).filter(
        (question) => {
          if (scope === ALL_PROJECTS || question.board) return true
          const session =
            sessionMap.get(question.sessionId) ??
            rootForSession.get(question.sessionId)?.session
          const card = cardMap.get(
            (question.ticketId ?? linkedTicket(session) ?? '')
              .trim()
              .toLowerCase(),
          )
          return card?.project_id === scope
        },
      ),
    [
      asksLive.data,
      cardMap,
      questionsLive.data,
      rootForSession,
      scope,
      sessionMap,
    ],
  )

  const basicItems = useMemo<NeedsYouItem[]>(() => {
    const openQuestions: NeedsYouItem[] = questions.flatMap((record) => {
      if (!['open', 'answering', 'expired'].includes(record.state)) return []
      const ask: AskEntry = record.harness ?? {
        session_id: record.sessionId,
        question_id: record.questionId,
        turn_id: '',
        asked_by: record.board?.comment.author,
        questions: record.questions,
        about: record.about,
        asked_at: record.askedAt,
        on_expiry: 'block',
        state: record.state as 'open' | 'answering' | 'expired',
      }
      const session =
        sessionMap.get(ask.session_id) ??
        rootForSession.get(ask.session_id)?.session
      const card = cardMap.get(
        (ask.about.ticket ?? record.ticketId ?? linkedTicket(session) ?? '')
          .trim()
          .toLowerCase(),
      )
      if (!record.board && !isInScope(scope, card?.project_id)) return []
      const blocks = ask.about.blocks ?? []
      return [
        {
          id: `question:${ask.session_id}:${ask.question_id}`,
          kind: 'question',
          sessionId: ask.session_id,
          questionId: ask.question_id,
          title: ask.questions[0]?.question ?? 'Question',
          question: ask.questions[0]?.question ?? '',
          questionClass: ask.about.class ?? 'scope',
          why: ask.about.why,
          blocks,
          blockedCards: blocks.length,
          createdAt: ask.asked_at,
          state: ask.state as 'open' | 'answering' | 'expired',
          entry: ask,
          sessionTitle: session?.title,
          currentTurnId: statusMap.get(ask.session_id)?.turn_id ?? null,
          card,
        },
      ]
    })
    const held: NeedsYouItem[] = (holdsLive.data ?? []).flatMap((pending) => {
      const session =
        sessionMap.get(pending.session_id) ??
        rootForSession.get(pending.session_id)?.session
      const ticket =
        pending.session_metadata?.kanban_ticket ?? linkedTicket(session)
      const card =
        typeof ticket === 'string'
          ? cardMap.get(ticket.trim().toLowerCase())
          : undefined
      if (!isInScope(scope, card?.project_id)) return []
      return [
        {
          id: `held:${pending.session_id}:${pending.function_call_id}`,
          kind: 'held',
          sessionId: pending.session_id,
          functionCallId: pending.function_call_id,
          functionId: pending.function_id,
          arguments: pending.arguments_excerpt,
          title: pending.function_id,
          sessionTitle: pending.session_title ?? session?.title,
          namespace: pending.namespace,
          implicitNamespace: pending.implicit_namespace,
          blockedCards: card ? 1 : 0,
          createdAt: pending.pending_at,
          card,
        },
      ]
    })
    return [...openQuestions, ...held]
  }, [
    cardMap,
    holdsLive.data,
    questions,
    rootForSession,
    scope,
    sessionMap,
    statusMap,
  ])

  const runsBeforeNeeds = useMemo<TowerRun[]>(() => {
    const now = clock
    return (sessionsLive.data?.rows ?? []).flatMap((row) => {
      const card = cardForSession(row.session, cards)
      if (!isInScope(scope, card?.project_id)) return []
      const lastProgress = newestProgress(row.session, card, row.subtasks ?? [])
      const openQuestions = row.status.open_asks + row.childOpenAsks
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
          sessionsLive.data?.stallAfterMin,
        )
      const hasHold = basicItems.some(
        (item) =>
          item.kind === 'held' &&
          (item.sessionId === row.session.session_id ||
            row.descendantSessionIds.includes(item.sessionId)),
      )
      const active = row.session.status === 'working' || row.runningChildren > 0
      const linkedRecently =
        Boolean(card) && now - row.session.updated_at <= 24 * 60 * 60_000
      if (
        !active &&
        !openQuestions &&
        !hasHold &&
        !row.status.expects_wake &&
        !linkedRecently
      ) {
        return []
      }
      return [
        {
          session: row.session,
          status: row.status,
          card,
          subtasks: row.subtasks,
          progress: progressFor(
            row.subtasks ?? [],
            releaseCatalog,
            boardLive.data?.board.doneStatus ?? 'done',
            boardLive.data?.board.canceledStatus,
          ),
          needsYou: [],
          lastProgress,
          lastAssistant: row.lastAssistant,
          state: stalled
            ? 'stalled'
            : row.session.status === 'error'
              ? 'failed'
              : active
                ? 'working'
                : openQuestions || hasHold || row.status.expects_wake
                  ? 'waiting'
                  : 'done',
          profile: (() => {
            const profile = row.session.metadata?.agent_profile
            if (typeof profile === 'string') return profile
            if (profile && typeof profile === 'object') {
              const id = (profile as Record<string, unknown>).id
              return typeof id === 'string' ? id : undefined
            }
            return undefined
          })(),
          startedBy: row.session.session_id.startsWith('dispatch-')
            ? 'board dispatch'
            : row.session.kind === 'automation'
              ? 'automation'
              : 'chat',
          workingDir: fsRoot(row.session),
          descendantSessionIds: row.descendantSessionIds,
          metrics: row.metrics,
        },
      ]
    })
  }, [
    basicItems,
    boardLive.data?.board,
    cards,
    clock,
    releaseCatalog,
    scope,
    sessionsLive.data,
  ])

  const stalledItems = useMemo<NeedsYouItem[]>(
    () =>
      runsBeforeNeeds
        .filter((run) => run.state === 'stalled')
        .map((run) => ({
          id: `stalled:${run.session.session_id}`,
          kind: 'stalled',
          sessionId: run.session.session_id,
          title: run.session.title,
          blockedCards: run.card ? 1 : 0,
          createdAt: run.lastProgress,
          lastProgress: run.lastProgress,
          armedWakes: run.status.armed_wakes,
          card: run.card,
        })),
    [runsBeforeNeeds],
  )
  const items = useMemo(
    () => sortNeeds([...basicItems, ...stalledItems]),
    [basicItems, stalledItems],
  )
  const runs = useMemo(
    () =>
      runsBeforeNeeds.map((run) => ({
        ...run,
        needsYou: items.filter(
          (item) =>
            item.sessionId === run.session.session_id ||
            run.descendantSessionIds.includes(item.sessionId) ||
            (item.card !== undefined && item.card.id === run.card?.id),
        ),
      })),
    [items, runsBeforeNeeds],
  )

  const [repositories, setRepositories] = useState<string[]>([])
  const repositoryRoots = useMemo(
    () =>
      runs.map((run) => ({
        workingDir: run.workingDir,
        projectId: run.card?.project_id,
      })),
    [runs],
  )
  const repositoryInputKey = JSON.stringify({
    scope,
    projects: projects.map((project) => ({
      id: project.id,
      repos: project.repos ?? [],
    })),
    roots: repositoryRoots,
  })
  const [repositoryError, setRepositoryError] = useState<string | null>(null)
  const [repositoriesLoading, setRepositoriesLoading] = useState(true)
  const [repositoryToken, setRepositoryToken] = useState(0)
  useEffect(() => {
    void repositoryToken
    let current = true
    setRepositoriesLoading(true)
    const input = JSON.parse(repositoryInputKey) as {
      scope: string
      projects: TowerProject[]
      roots: Array<{ workingDir?: string; projectId?: string | null }>
    }
    void repositoriesFor(iii, input.scope, input.projects, input.roots).then(
      (next) => {
        if (!current) return
        setRepositories((previous) =>
          previous.length === next.length &&
          previous.every((repository, index) => repository === next[index])
            ? previous
            : next,
        )
        setRepositoryError(null)
        setRepositoriesLoading(false)
      },
      (reason) => {
        if (!current) return
        setRepositoryError(errorMessage(reason))
        setRepositoriesLoading(false)
      },
    )
    return () => {
      current = false
    }
  }, [iii, repositoryInputKey, repositoryToken])
  const fetchPrs = useCallback(
    () => fetchPullRequests(iii, repositories, []),
    [iii, repositories],
  )
  const prLive = useLiveSource(iii, 'iii::console::tower::prs', [], fetchPrs)
  useEffect(() => {
    const timer = window.setInterval(prLive.refresh, 5 * 60_000)
    return () => window.clearInterval(timer)
  }, [prLive.refresh])

  const refresh = useCallback(() => {
    boardLive.refresh()
    sessionsLive.refresh()
    asksLive.refresh()
    holdsLive.refresh()
    questionsLive.refresh()
    setRepositoryToken((token) => token + 1)
    prLive.refresh()
  }, [
    asksLive.refresh,
    boardLive.refresh,
    holdsLive.refresh,
    questionsLive.refresh,
    prLive.refresh,
    sessionsLive.refresh,
  ])
  const prs = useMemo(
    () =>
      (prLive.data ?? []).map((pr) => ({
        ...pr,
        card: linkedCard(pr.title, pr.headRefName, cards),
      })),
    [cards, prLive.data],
  )

  return {
    projects,
    board: boardLive.data?.board ?? null,
    tickets: cards,
    releases,
    releaseCatalog,
    runs,
    items,
    questions,
    prs,
    repositories,
    loading: {
      board: boardLive.loading,
      sessions: sessionsLive.loading,
      needsYou:
        asksLive.loading ||
        holdsLive.loading ||
        questionsLive.loading ||
        boardLive.loading ||
        sessionsLive.loading,
      questions: asksLive.loading || questionsLive.loading,
      prs: repositoriesLoading || prLive.loading,
    },
    errors: {
      board: boardLive.error,
      sessions: sessionsLive.error,
      needsYou: asksLive.error ?? holdsLive.error ?? questionsLive.error,
      questions: asksLive.error ?? questionsLive.error,
      prs: repositoryError ?? prLive.error,
    },
    refresh,
  }
}
