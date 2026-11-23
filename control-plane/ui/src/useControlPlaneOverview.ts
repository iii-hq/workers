import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { ALL_PROJECTS, isInScope } from '../../src/model'
import { summaryIsFresh } from '../../src/summary'
import type { HeldNeed, ControlPlaneData, ControlPlaneQuestion } from '../../src/types'
import type { RefreshResult, WriterSnapshot } from '../../src/writer'

export interface SummaryState extends WriterSnapshot {
  fresh: boolean
  loading: boolean
  refreshing: boolean
  throttled_until?: string
  refresh(): void
  now: number
}
export interface ControlPlaneOverviewData extends ControlPlaneData {
  scope: string
  refresh(): void
  summary: SummaryState
  availability: { github: boolean; approval_gate: boolean }
  assumptionsSinceVisit: ControlPlaneQuestion[]
  prs_interval_min: number
  decidedCursor?: string
  loadMoreDecided(): void
}
type Overview = ControlPlaneData & {
  summary: WriterSnapshot & { fresh: boolean }
  availability: ControlPlaneOverviewData['availability']
  prs_interval_min: number
}
interface PendingApproval {
  session_id: string
  function_call_id: string
  function_id: string
  namespace?: string
  implicit_namespace?: string
  arguments_excerpt: unknown
  pending_at: number
  session_title?: string
  session_metadata?: Record<string, unknown>
}
type Part = 'overview' | 'needs' | 'runs' | 'prs' | 'decided' | 'assumptions' | 'holds'
const PARTS: Part[] = ['overview', 'needs', 'runs', 'prs', 'decided', 'assumptions', 'holds']
const idleSummary: Overview['summary'] = {
  result: null,
  valid_hash: null,
  checked_at: null,
  error: null,
  unavailable: false,
  fresh: false,
}
function emptyData(): ControlPlaneData {
  return {
    as_of: '',
    sourceHash: '',
    projects: [],
    board: null,
    tickets: [],
    releases: [],
    releaseCatalog: [],
    runs: [],
    items: [],
    questions: [],
    prs: [],
    repositories: [],
    direction: [],
    median_decision_wait_ms: null,
    exceptions: { stalled: [], failing_prs: [], blocked_cards: [], old_questions: [] },
    loading: { board: true, sessions: true, needsYou: true, questions: true, prs: true },
    errors: { board: null, sessions: null, needsYou: null, questions: null, prs: null },
  }
}

// The console owns approval reads; they never pass through control-plane::*.
export async function fetchControlPlaneHolds(iii: Host['iii']): Promise<PendingApproval[]> {
  const pending: PendingApproval[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{ pending: PendingApproval[]; next_cursor?: string }>('approval::list-pending', {
      limit: 200,
      ...(cursor ? { cursor } : {}),
    })
    pending.push(...page.pending)
    cursor = page.next_cursor
  } while (cursor)
  return pending
}

export function mergeHolds(data: ControlPlaneData, holds: PendingApproval[], scope: string): ControlPlaneData {
  const held = holds.flatMap((pending): Array<HeldNeed & { urgency: 'now' }> => {
    const run = data.runs.find(
      (row) => row.session.session_id === pending.session_id || row.descendantSessionIds.includes(pending.session_id),
    )
    const ticket = pending.session_metadata?.kanban_ticket ?? run?.card?.key
    const card =
      typeof ticket === 'string'
        ? data.tickets.find((row) => row.id === ticket.trim() || row.key.toLowerCase() === ticket.trim().toLowerCase())
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
        sessionTitle: pending.session_title ?? run?.session.title,
        namespace: pending.namespace,
        implicitNamespace: pending.implicit_namespace,
        createdAt: pending.pending_at,
        blockedCards: card ? 1 : 0,
        card,
        urgency: 'now',
      },
    ]
  })
  const items = [...data.items, ...held].sort(
    (a, b) =>
      Number(b.urgency === 'now') - Number(a.urgency === 'now') ||
      b.blockedCards - a.blockedCards ||
      a.createdAt - b.createdAt,
  )
  const runs = data.runs.map((run) => {
    const needsYou = items.filter(
      (item) =>
        item.sessionId === run.session.session_id ||
        run.descendantSessionIds.includes(item.sessionId) ||
        Boolean(item.card && item.card.id === run.card?.id),
    )
    const parkedWithHold =
      (run.state === 'done' || (run.state === 'working' && run.status.status === 'awaiting_functions')) &&
      needsYou.some(
        (item) =>
          item.kind === 'held' &&
          (item.sessionId === run.session.session_id || run.descendantSessionIds.includes(item.sessionId)),
      )
    return {
      ...run,
      needsYou,
      state: parkedWithHold ? ('waiting' as const) : run.state,
    }
  })
  return { ...data, items, runs }
}

export function useControlPlaneOverview(host: Host, scope = ALL_PROJECTS, since?: string): ControlPlaneOverviewData {
  const iii = host.iii
  const id = useId().replace(/[^a-z0-9]/gi, '')
  const refetchAll = useRef<() => void>(() => {})
  const [snapshot, setSnapshot] = useState<{
    scope: string
    data: ControlPlaneData
    summary: Overview['summary']
    availability: Overview['availability']
    interval: number
  }>(() => ({
    scope,
    data: emptyData(),
    summary: idleSummary,
    availability: { github: false, approval_gate: false },
    interval: 5,
  }))
  const [holds, setHolds] = useState<PendingApproval[]>([])
  const [holdsLoading, setHoldsLoading] = useState(false)
  const [sourceErrors, setSourceErrors] = useState<Partial<Record<Part, string | null>>>({})
  const [assumptions, setAssumptions] = useState<ControlPlaneQuestion[]>([])
  const [decided, setDecided] = useState<ControlPlaneQuestion[]>([])
  const [assumptionsSinceVisit, setAssumptionsSinceVisit] = useState<ControlPlaneQuestion[]>([])
  const [decidedCursor, setDecidedCursor] = useState<string>()
  const [refreshing, setRefreshing] = useState(false)
  const [throttled, setThrottled] = useState<string>()
  const [now, setNow] = useState(Date.now)
  const [loadNext, setLoadNext] = useState<() => void>(() => () => {})
  const refresh = useCallback(() => refetchAll.current(), [])

  useEffect(() => {
    let active = true
    let approvalAvailable = false
    let holdObservers: Array<() => void> = []
    const sequence: Partial<Record<Part, number>> = {}
    setSnapshot({
      scope,
      data: emptyData(),
      summary: idleSummary,
      availability: { github: false, approval_gate: false },
      interval: 5,
    })
    setHolds([])
    setHoldsLoading(false)
    setSourceErrors({})
    setAssumptions([])
    setDecided([])
    setAssumptionsSinceVisit([])
    setDecidedCursor(undefined)
    setThrottled(undefined)
    const update = (patch: Partial<ControlPlaneData>) =>
      setSnapshot((previous) => ({ ...previous, data: { ...previous.data, ...patch } }))
    const fetchPart = async (part: Part, cursor?: string) => {
      const mine = (sequence[part] = (sequence[part] ?? 0) + 1)
      const current = () => active && mine === sequence[part]
      try {
        if (part === 'overview') {
          const versions = { ...sequence }
          const next = await iii.call<Overview>('control-plane::overview', { scope })
          if (!current()) return
          approvalAvailable = next.availability.approval_gate
          if (approvalAvailable && !holdObservers.length) {
            holdObservers = ['approval::on-pending-created', 'approval::on-pending-resolved'].map((source) =>
              iii.observe(source, `control-plane-ui-holds-${id}::${iii.browserId}`),
            )
          } else if (!approvalAvailable) {
            for (const dispose of holdObservers) dispose()
            holdObservers = []
          }
          setSnapshot((previous) => ({
            scope,
            data: {
              ...next,
              ...(sequence.needs !== versions.needs ? { items: previous.data.items } : {}),
              ...(sequence.runs !== versions.runs ? { runs: previous.data.runs } : {}),
              ...(sequence.prs !== versions.prs ? { prs: previous.data.prs } : {}),
              ...(sequence.decided !== versions.decided
                ? { median_decision_wait_ms: previous.data.median_decision_wait_ms }
                : {}),
              errors: {
                ...next.errors,
                ...(sequence.needs !== versions.needs ? { needsYou: previous.data.errors.needsYou } : {}),
                ...(sequence.runs !== versions.runs ? { sessions: previous.data.errors.sessions } : {}),
                ...(sequence.prs !== versions.prs ? { prs: previous.data.errors.prs } : {}),
              },
            },
            summary: next.summary,
            availability: next.availability,
            interval: next.prs_interval_min,
          }))
          if (approvalAvailable) void fetchPart('holds')
          else {
            sequence.holds = (sequence.holds ?? 0) + 1
            setHoldsLoading(false)
            setHolds([])
            setSourceErrors((previous) => ({ ...previous, holds: null }))
          }
        } else if (part === 'needs') {
          const next = await iii.call<Pick<ControlPlaneData, 'items'>>('control-plane::needs-you::list', { scope })
          if (current())
            setSnapshot((previous) => ({
              ...previous,
              data: {
                ...previous.data,
                loading: { ...previous.data.loading, needsYou: false },
                items: next.items,
                errors: { ...previous.data.errors, needsYou: null },
              },
            }))
        } else if (part === 'runs' || part === 'prs') {
          const next = await iii.call<{ runs: ControlPlaneData['runs']; prs: ControlPlaneData['prs']; error: string | null }>(
            `control-plane::${part}::list`,
            { scope },
          )
          if (current())
            setSnapshot((previous) => ({
              ...previous,
              data: {
                ...previous.data,
                [part]: next[part],
                loading: { ...previous.data.loading, [part === 'runs' ? 'sessions' : 'prs']: false },
                errors: { ...previous.data.errors, [part === 'runs' ? 'sessions' : 'prs']: next.error },
              },
            }))
        } else if (part === 'decided') {
          const next = await iii.call<{
            questions: ControlPlaneQuestion[]
            cursor?: string
            median_decision_wait_ms: number | null
          }>('control-plane::decided::list', { scope, ...(cursor ? { cursor } : {}) })
          if (!current()) return
          setDecided((previous) => (cursor ? [...previous, ...next.questions] : next.questions))
          setDecidedCursor(next.cursor)
          setLoadNext(() => () => {
            if (next.cursor) void fetchPart('decided', next.cursor)
          })
          update({ median_decision_wait_ms: next.median_decision_wait_ms })
        } else if (part === 'assumptions') {
          const [all, recent] = await Promise.all([
            iii.call<{ questions: ControlPlaneQuestion[] }>('control-plane::assumptions::list', { scope }),
            since
              ? iii.call<{ questions: ControlPlaneQuestion[] }>('control-plane::assumptions::list', { scope, since })
              : Promise.resolve({ questions: [] as ControlPlaneQuestion[] }),
          ])
          if (!current()) return
          setAssumptions(all.questions)
          setAssumptionsSinceVisit(since ? recent.questions : all.questions)
        } else if (approvalAvailable) {
          setHoldsLoading(true)
          const pending = await fetchControlPlaneHolds(iii)
          if (current()) {
            setHolds(pending)
            setSourceErrors((previous) => ({ ...previous, holds: null }))
          }
        }
        if (current()) setSourceErrors((previous) => ({ ...previous, [part]: null }))
      } catch (error) {
        if (!current()) return
        setSourceErrors((previous) => ({ ...previous, [part]: errorMessage(error) }))
        const field =
          part === 'prs'
            ? 'prs'
            : part === 'runs'
              ? 'sessions'
              : part === 'decided' || part === 'assumptions'
                ? 'questions'
                : part === 'needs' || part === 'holds'
                  ? 'needsYou'
                  : 'board'
        setSnapshot((previous) => ({
          ...previous,
          data: {
            ...previous.data,
            loading: { ...previous.data.loading, [field]: false },
          },
        }))
      } finally {
        if (part === 'holds' && current()) setHoldsLoading(false)
      }
    }
    const refetch = (parts: Part[]) => {
      for (const part of parts) void fetchPart(part)
    }
    const handler = `control-plane-ui-${id}`
    const off = iii.on<{ scope: string; parts: string[] }>(handler, (event) => {
      if (event.scope !== scope) return
      const changed = new Set<Part>()
      for (const part of event.parts) {
        if (part === 'items') changed.add('needs')
        else if (part === 'runs') changed.add('runs')
        else if (part === 'prs') changed.add('prs')
        else if (part === 'questions') {
          changed.add('overview')
          changed.add('decided')
          changed.add('assumptions')
        } else changed.add('overview')
      }
      refetch([...changed])
    })
    const offHolds = iii.on(`control-plane-ui-holds-${id}`, () => {
      void fetchPart('holds')
    })
    const unobserve = iii.observe('control-plane::on-changed', `${handler}::${iii.browserId}`)
    const watch = () => {
      setNow(Date.now())
      void iii.call('control-plane::watch', { scope }).catch(() => {})
    }
    const reconnect = iii.addConnectionStateListener((state) => {
      if (state !== 'connected') return
      refetch(PARTS)
      watch()
    })
    refetchAll.current = () => refetch(PARTS)
    refetch(PARTS.filter((part) => part !== 'holds'))
    watch()
    const timer = setInterval(watch, 60_000)
    return () => {
      active = false
      refetchAll.current = () => {}
      clearInterval(timer)
      reconnect()
      unobserve()
      for (const dispose of holdObservers) dispose()
      offHolds()
      off()
    }
  }, [iii, scope, since, id])

  const refreshSummary = useCallback(() => {
    setRefreshing(true)
    void iii
      .call<RefreshResult>('control-plane::summary::refresh', { scope })
      .then(
        (result) => {
          setThrottled(result.throttled_until)
          setSnapshot((previous) =>
            previous.scope === scope
              ? {
                  ...previous,
                  summary: {
                    ...result,
                    fresh: !result.error && result.checked_at != null && summaryIsFresh(Date.parse(result.checked_at)),
                  },
                }
              : previous,
          )
        },
        (error) => {
          setSnapshot((previous) =>
            previous.scope === scope
              ? { ...previous, summary: { ...previous.summary, error: errorMessage(error), fresh: false } }
              : previous,
          )
        },
      )
      .finally(() => setRefreshing(false))
  }, [iii, scope])

  return useMemo(() => {
    const data = snapshot.scope === scope ? snapshot.data : emptyData()
    const merged = mergeHolds(data, snapshot.scope === scope ? holds : [], scope)
    const questions = new Map(
      [...assumptions, ...decided, ...data.questions].map((question) => [question.key, question]),
    )
    return {
      ...merged,
      loading: {
        ...merged.loading,
        needsYou: merged.loading.needsYou || (snapshot.availability.approval_gate && holdsLoading),
      },
      errors: {
        board: sourceErrors.overview ?? merged.errors.board,
        sessions: sourceErrors.runs ?? merged.errors.sessions,
        needsYou: sourceErrors.holds ?? sourceErrors.needs ?? merged.errors.needsYou,
        questions: sourceErrors.decided ?? sourceErrors.assumptions ?? merged.errors.questions,
        prs: sourceErrors.prs ?? merged.errors.prs,
      },
      scope,
      questions: [...questions.values()].sort((a, b) => b.askedAt - a.askedAt),
      refresh,
      summary: {
        ...snapshot.summary,
        loading: data.loading.board,
        refreshing,
        throttled_until: throttled,
        refresh: refreshSummary,
        now,
      },
      availability: snapshot.availability,
      assumptionsSinceVisit,
      prs_interval_min: snapshot.interval,
      decidedCursor,
      loadMoreDecided: loadNext,
    }
  }, [
    snapshot,
    scope,
    holds,
    holdsLoading,
    sourceErrors,
    assumptions,
    decided,
    assumptionsSinceVisit,
    decidedCursor,
    loadNext,
    refresh,
    refreshSummary,
    refreshing,
    throttled,
    now,
  ])
}
