import { Button, StatusPanel } from '@iii-dev/console-ui'
import { monitorSummary } from '../../model'
import type { AnalysisRecord, MonitorState } from '../../types'
import { capacityNotice, clock, pausedDetail } from './shell-state'

/**
 * Monitor-level notices, directly under the status card so they are seen
 * before Analyze: the monitor did not answer, the observer did not bind,
 * observation is paused, or a session was turned away at capacity.
 */
export function MonitorNotices({
  state,
  loadError,
  records,
  now,
  narrow,
  onRetry,
}: {
  state: MonitorState | null
  /** The last read of the monitor failed (an older answer may still be held). */
  loadError: string | null
  records: AnalysisRecord[] | null
  now: number
  narrow: boolean
  onRetry: () => void
}) {
  const { running, pending } = monitorSummary(records ?? [], now)
  const capacity = capacityNotice(state, records, now)
  const retry = (
    <Button variant="pill" size={narrow ? 'lg' : 'sm'} onClick={onRetry}>
      Retry
    </Button>
  )

  return (
    <>
      {loadError ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="Monitor unavailable"
          detail={
            <>
              The eval worker didn't answer. <code className="eval-ui-mono">{loadError}</code>
            </>
          }
          action={retry}
        />
      ) : null}
      {state && !state.observer_bound && !loadError ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="Monitor unavailable"
          detail={
            <>
              The <code className="eval-ui-mono">harness::turn-completed</code> trigger didn't register, so finished
              sessions aren't being seen.
              {state.observer_error ? (
                <>
                  {' '}
                  <code className="eval-ui-mono">{state.observer_error}</code>
                </>
              ) : null}
            </>
          }
          action={retry}
        />
      ) : null}
      {state?.config && !state.config.enabled ? (
        <StatusPanel
          variant="warn"
          role="status"
          headline="Observation paused"
          detail={pausedDetail(running, pending)}
        />
      ) : null}
      {capacity?.live ? (
        <StatusPanel
          variant="warn"
          role="status"
          headline="At capacity"
          detail={
            <>
              {capacity.unfinished} analyses are unfinished. <code className="eval-ui-mono">{capacity.sessionId}</code>{' '}
              wasn't admitted and no model was called.
            </>
          }
        />
      ) : null}
      {capacity && !capacity.live ? (
        <StatusPanel
          variant="warn"
          role="status"
          headline="A session was turned away"
          detail={
            <>
              At {clock(capacity.at)} the monitor was at capacity.{' '}
              <code className="eval-ui-mono">{capacity.sessionId}</code> wasn't admitted and no model was called.
            </>
          }
        />
      ) : null}
    </>
  )
}
