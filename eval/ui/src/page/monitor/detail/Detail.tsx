// One analysis, from masthead to evidence. Loads the result, follows it every
// 2 s while it is running, and keeps its own clock for elapsed and remaining.
import {
  Button,
  EmptyState,
  type Host,
  type LiveAnnouncement,
  LiveRegion,
  Skeleton,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { FileQuestion } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { EvalApi } from '../../../api'
import { isActive, statusPresentation } from '../../../model'
import type { AnalysisResult, EntryRef, MonitorLimits } from '../../../types'
import { canOpenSession, openSession } from '../open-session'
import { Evidence } from './Evidence'
import { Masthead } from './Masthead'
import { Pipeline } from './Pipeline'
import { Rail } from './Rail'
import { Signals } from './Signals'
import { StateNotice } from './StateNotice'
import { Suggestions } from './Suggestions'
import { scrollToElement } from './scroll'
import type { DetailActions, JumpTarget } from './shared'
import { Triage } from './Triage'

const POLL_MS = 2000

export interface AnalysisDetailProps {
  host: Host
  api: EvalApi
  evaluationId: string
  /** The pane is narrow: one column, Back, touch-sized controls. */
  narrow: boolean
  /** What the monitor enforces; `undefined` until its state is read, and the copy then names no number. */
  limits: MonitorLimits | undefined
  /** Bumps when the list re-reads or an analysis finished elsewhere. */
  refreshKey: number
  onBack?: () => void
  onSelect: (evaluationId: string) => void
  onChanged: () => void
  onDeleted: () => void
}

/** One skeleton block of a given size. */
function Sk({ w, h, block }: { w?: number | string; h?: number; block?: boolean }) {
  return <Skeleton className="eval-ui-ad-sk" data-block={block || undefined} style={{ width: w, height: h }} />
}

/** Skeleton of the screen that is coming: masthead, steps, a card, the rail. */
function DetailSkeleton({ narrow }: { narrow: boolean }) {
  return (
    <div
      className="eval-ui-ad"
      data-narrow={narrow || undefined}
      role="status"
      aria-busy="true"
      aria-label="Loading analysis"
    >
      <div className="eval-ui-ad-inner">
        <div className="eval-ui-ad-masthead">
          <div className="eval-ui-ad-masthead-copy">
            <div className="eval-ui-ad-ident">
              <Sk w={86} h={14} />
              <Sk w={150} h={22} />
              <Sk w={72} h={22} />
            </div>
            <Sk w="58%" h={26} />
            <Sk w="42%" h={14} />
          </div>
          {narrow ? null : (
            <div className="eval-ui-ad-actions">
              <Sk w={98} h={32} />
              <Sk w={88} h={32} />
              <Sk w={32} h={32} />
            </div>
          )}
        </div>
        <div className="eval-ui-ad-steps">
          {[0, 1, 2, 3, 4].map((step) => (
            <div key={step} className="eval-ui-ad-step">
              <Sk w={64} h={13} />
              <Sk w={44} h={11} />
            </div>
          ))}
        </div>
        <div className="eval-ui-ad-cols">
          <div className="eval-ui-ad-rail">
            <Sk block h={180} />
            <Sk block h={120} />
            <Sk block h={150} />
          </div>
          <div className="eval-ui-ad-main">
            <div className="eval-ui-ad-section">
              <Sk w={84} h={14} />
              <div className="eval-ui-ad-card">
                <Sk w={110} h={22} />
                <Sk w="72%" h={20} />
                {[0, 1, 2, 3].map((row) => (
                  <div key={row} className="eval-ui-ad-sk-lines">
                    <Sk w="100%" h={14} />
                    <Sk w="64%" h={14} />
                  </div>
                ))}
              </div>
            </div>
            <div className="eval-ui-ad-section">
              <Sk w={64} h={14} />
              <div className="eval-ui-ad-panel eval-ui-ad-signals">
                {[0, 1].map((row) => (
                  <div key={row} className="eval-ui-ad-signal">
                    <Sk w="36%" h={14} />
                    <Sk w="88%" h={14} />
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

/** The analysis changed meaningfully, or it is only the clock that moved. */
function sameRecord(a: AnalysisResult, b: AnalysisResult): boolean {
  return (
    a.record.updated_at === b.record.updated_at &&
    a.record.status === b.record.status &&
    a.record.step === b.record.step &&
    a.record.counters.validations === b.record.counters.validations
  )
}

function DetailBody({
  host,
  api,
  evaluationId,
  narrow,
  limits,
  refreshKey,
  onBack,
  onSelect,
  onChanged,
  onDeleted,
}: AnalysisDetailProps) {
  // `undefined` while the first read is out, `null` when the analysis is gone.
  const [result, setResult] = useState<AnalysisResult | null | undefined>(undefined)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [busy, setBusy] = useState<DetailActions['busy']>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const [jump, setJump] = useState<JumpTarget | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const { confirm, dialog } = useConfirm()
  const root = useRef<HTMLDivElement>(null)
  // Only the newest read may land: a slow answer must not paint over a newer one.
  const request = useRef(0)
  // A read that is still out: a poll waits for it, or it would never land.
  const inFlight = useRef(false)
  const jumps = useRef(0)
  const announced = useRef<string | null>(null)
  const announcements = useRef(0)

  const load = useCallback(
    async (quiet = false) => {
      if (quiet && inFlight.current) return
      const seq = ++request.current
      inFlight.current = true
      try {
        const next = await api.result(evaluationId)
        if (seq !== request.current) return
        setLoadError(null)
        // A poll that found nothing new keeps the objects, so nothing re-renders.
        setResult((previous) => (quiet && previous && next && sameRecord(previous, next) ? previous : next))
      } catch (cause) {
        if (seq === request.current) setLoadError(errorMessage(cause))
      } finally {
        if (seq === request.current) inFlight.current = false
      }
    },
    [api, evaluationId],
  )

  useEffect(() => {
    void load(false)
  }, [load, refreshKey])

  const running = result ? isActive(result.record.status) : false
  // The clock and the poll belong to an analysis that is running; a hidden
  // browser tab pauses both.
  const watching = running
  useEffect(() => {
    if (!watching) return
    setNow(Date.now())
    const clock = window.setInterval(() => {
      if (!document.hidden) setNow(Date.now())
    }, 1000)
    const poll = window.setInterval(() => {
      if (!document.hidden) void load(true)
    }, POLL_MS)
    const onVisible = () => {
      if (document.hidden) return
      setNow(Date.now())
      void load(true)
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      window.clearInterval(clock)
      window.clearInterval(poll)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [watching, load])

  const announce = useCallback((text: string, urgency: LiveAnnouncement['urgency'] = 'polite') => {
    announcements.current += 1
    setAnnouncement({ seq: announcements.current, text, urgency })
  }, [])

  // Say it once when the analysis moves to another state.
  const label = result ? statusPresentation(result.record).label : null
  useEffect(() => {
    if (label === null) return
    if (announced.current !== null && announced.current !== label) announce(`Analysis ${label}`)
    announced.current = label
  }, [announce, label])

  const run = useCallback(async (kind: NonNullable<DetailActions['busy']>, work: () => Promise<void>) => {
    setBusy(kind)
    setActionError(null)
    try {
      await work()
    } catch (cause) {
      setActionError(errorMessage(cause))
    } finally {
      setBusy(null)
    }
  }, [])

  /** The console took no way of showing it: say so instead of doing nothing. */
  const show = useCallback(
    (id: string) => {
      if (!openSession(host, id)) setActionError("This console can't open that session.")
    },
    [host],
  )

  /** The E2E console page, to run the scenario the plan asks for. */
  const openE2e = useCallback(() => {
    try {
      host.panels?.openScreen?.({ screen: 'ext:harness-e2e' })
    } catch {
      setActionError("This console can't open the E2E page.")
    }
  }, [host])

  const record = result?.record
  const canOpen = canOpenSession(host)
  const sessionId = record?.session_id
  const investigationSession = result?.assets.investigation?.session_id ?? record?.analyst?.session_id

  const actions: DetailActions = useMemo(
    () => ({
      busy,
      cancel: () =>
        void run('cancel', async () => {
          // It may have finished first (`cancelled: false`): the state read next, and
          // announced when its label changes, says what really happened.
          await api.cancel(evaluationId)
          await load(false)
          onChanged()
        }),
      reanalyze: () =>
        void run('reanalyze', async () => {
          if (!sessionId) return
          const started = await api.analyze(sessionId, true)
          announce('Reanalysis started')
          onChanged()
          onSelect(started.evaluation_id)
        }),
      remove: () =>
        void run('delete', async () => {
          const accepted = await confirm({
            title: 'Delete this analysis?',
            description:
              "Its signals, suggestions and linked E2E runs are removed. The session itself isn't touched, and this turn won't be analyzed again automatically. Analyze it by ID if you need it again.",
            confirmLabel: 'Delete',
            tone: 'danger',
          })
          if (!accepted) return
          await api.delete(evaluationId)
          onChanged()
          onDeleted()
        }),
      openSession: canOpen && sessionId ? () => show(sessionId) : undefined,
      openInvestigation: canOpen && investigationSession ? () => show(investigationSession) : undefined,
      viewSignals: () => scrollToElement(root.current?.querySelector<HTMLElement>('[data-section="signals"]')),
    }),
    [
      api,
      announce,
      busy,
      canOpen,
      confirm,
      evaluationId,
      investigationSession,
      load,
      onChanged,
      onDeleted,
      onSelect,
      run,
      sessionId,
      show,
    ],
  )

  const onJump = useCallback((entry: EntryRef) => {
    jumps.current += 1
    setJump({ entry, n: jumps.current })
  }, [])

  if (result === undefined) {
    if (loadError) {
      return (
        <div className="eval-ui-ad" data-narrow={narrow || undefined}>
          <div className="eval-ui-ad-inner">
            <StatusPanel
              variant="alert"
              role="alert"
              headline="Couldn't load this analysis"
              detail={loadError}
              action={
                <Button size={narrow ? 'lg' : 'sm'} variant="pill" onClick={() => void load(false)}>
                  Retry
                </Button>
              }
            />
          </div>
        </div>
      )
    }
    return <DetailSkeleton narrow={narrow} />
  }

  if (result === null || !record) {
    return (
      <div className="eval-ui-ad" data-narrow={narrow || undefined}>
        <EmptyState
          icon={FileQuestion}
          title="This analysis no longer exists"
          description="It was deleted, or removed once it passed the retention period. Pick another one from the list."
          action={onBack ? { label: 'Back to the list', onClick: onBack } : undefined}
        />
      </div>
    )
  }

  const { assets } = result
  const terminal = !running
  const snapshot = assets.snapshot

  const rail = <Rail record={record} assets={assets} openInvestigation={actions.openInvestigation} />
  const main = (
    <div className="eval-ui-ad-main">
      <Suggestions
        api={api}
        evaluationId={evaluationId}
        assets={assets}
        narrow={narrow}
        terminal={terminal}
        onJump={onJump}
        onOpenE2e={host.panels?.openScreen ? openE2e : undefined}
        onAttached={() => {
          void load(false)
          onChanged()
        }}
      />
      {snapshot ? (
        <Signals
          snapshot={snapshot}
          investigation={assets.investigation}
          terminal={terminal}
          jump={jump}
          narrow={narrow}
        />
      ) : null}
      <Triage record={record} assets={assets} limits={limits} />
      {snapshot ? <Evidence snapshot={snapshot} jump={jump} limits={limits} /> : null}
    </div>
  )

  return (
    <div ref={root} className="eval-ui-ad" data-narrow={narrow || undefined}>
      <div className="eval-ui-ad-inner">
        <Masthead record={record} snapshot={snapshot} narrow={narrow} actions={actions} onBack={onBack} />
        {actionError ? (
          <StatusPanel
            variant="alert"
            role="alert"
            headline="That did not go through"
            detail={actionError}
            action={
              <Button size={narrow ? 'lg' : 'sm'} variant="ghost" onClick={() => setActionError(null)}>
                Dismiss
              </Button>
            }
          />
        ) : null}
        {loadError ? (
          <StatusPanel
            variant="warn"
            role="alert"
            headline="Couldn't refresh this analysis"
            detail={`Showing what was read last. ${loadError}`}
            action={
              <Button size={narrow ? 'lg' : 'sm'} variant="pill" onClick={() => void load(false)}>
                Retry
              </Button>
            }
          />
        ) : null}
        <Pipeline record={record} now={now} narrow={narrow} />
        <StateNotice result={result} now={now} narrow={narrow} limits={limits} actions={actions} />
        <div className="eval-ui-ad-cols">
          {/* The rail reads first in every layout; the stylesheet puts it beside the sections when there is room. */}
          {rail}
          {main}
        </div>
      </div>
      <LiveRegion announcement={announcement} />
      {dialog}
    </div>
  )
}

export function AnalysisDetail(props: AnalysisDetailProps) {
  // A new analysis starts from a clean slate: loading, no stale errors.
  return <DetailBody key={props.evaluationId} {...props} />
}
