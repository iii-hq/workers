import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { ALL_PROJECTS } from '../../src/model'
import { summaryIsFresh } from '../../src/summary'
import type { ControlPlaneData, ControlPlaneQuestion } from '../../src/types'
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
  availability: { github: boolean }
  prs_interval_min: number
  decidedCursor?: string
  loadMoreDecided(): void
}
type Overview = ControlPlaneData & {
  summary: WriterSnapshot & { fresh: boolean }
  availability: ControlPlaneOverviewData['availability']
  prs_interval_min: number
}
type Part = 'overview' | 'needs' | 'runs' | 'prs' | 'decided' | 'assumptions'
const PARTS: Part[] = ['overview', 'needs', 'runs', 'prs', 'decided', 'assumptions']
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
    median_decision_wait_ms: null,
    loading: { board: true, sessions: true, needsYou: true, questions: true, prs: true },
    errors: { board: null, sessions: null, needsYou: null, questions: null, prs: null },
  }
}

export function useControlPlaneOverview(host: Host, scope = ALL_PROJECTS): ControlPlaneOverviewData {
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
    availability: { github: false },
    interval: 5,
  }))
  const [sourceErrors, setSourceErrors] = useState<Partial<Record<Part, string | null>>>({})
  const [assumptions, setAssumptions] = useState<ControlPlaneQuestion[]>([])
  const [decided, setDecided] = useState<ControlPlaneQuestion[]>([])
  const [decidedCursor, setDecidedCursor] = useState<string>()
  const [refreshing, setRefreshing] = useState(false)
  const [throttled, setThrottled] = useState<string>()
  const [now, setNow] = useState(Date.now)
  const [loadNext, setLoadNext] = useState<() => void>(() => () => {})
  const refresh = useCallback(() => refetchAll.current(), [])

  useEffect(() => {
    let active = true
    const sequence: Partial<Record<Part, number>> = {}
    setSnapshot({
      scope,
      data: emptyData(),
      summary: idleSummary,
      availability: { github: false },
      interval: 5,
    })
    setSourceErrors({})
    setAssumptions([])
    setDecided([])
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
          const all = await iii.call<{ questions: ControlPlaneQuestion[] }>('control-plane::assumptions::list', {
            scope,
          })
          if (!current()) return
          setAssumptions(all.questions)
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
                : part === 'needs'
                  ? 'needsYou'
                  : 'board'
        setSnapshot((previous) => ({
          ...previous,
          data: {
            ...previous.data,
            loading: { ...previous.data.loading, [field]: false },
          },
        }))
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
    refetch(PARTS)
    watch()
    const timer = setInterval(watch, 60_000)
    return () => {
      active = false
      refetchAll.current = () => {}
      clearInterval(timer)
      reconnect()
      unobserve()
      off()
    }
  }, [iii, scope, id])

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
    const questions = new Map(
      [...assumptions, ...decided, ...data.questions].map((question) => [question.key, question]),
    )
    return {
      ...data,
      errors: {
        board: sourceErrors.overview ?? data.errors.board,
        sessions: sourceErrors.runs ?? data.errors.sessions,
        needsYou: sourceErrors.needs ?? data.errors.needsYou,
        questions: sourceErrors.decided ?? sourceErrors.assumptions ?? data.errors.questions,
        prs: sourceErrors.prs ?? data.errors.prs,
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
      prs_interval_min: snapshot.interval,
      decidedCursor,
      loadMoreDecided: loadNext,
    }
  }, [
    snapshot,
    scope,
    sourceErrors,
    assumptions,
    decided,
    decidedCursor,
    loadNext,
    refresh,
    refreshSummary,
    refreshing,
    throttled,
    now,
  ])
}
