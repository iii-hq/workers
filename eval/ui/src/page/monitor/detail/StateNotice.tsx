// What the monitor is doing, or exactly why it stopped, under the steps.
import { Button, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { Ban, CircleAlert, CircleCheck, CircleHelp, Clock, FileX, LoaderCircle, TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import type { AnalysisResult, MonitorLimits } from '../../../types'
import { describeState, type NoticeAction, type NoticeIcon, type NoticeTone } from './notices'
import type { DetailActions } from './shared'

const VARIANT: Record<NoticeTone, 'info' | 'warn' | 'alert'> = {
  info: 'info',
  running: 'info',
  warn: 'warn',
  alert: 'alert',
}

function Glyph({ icon, tone }: { icon: NoticeIcon; tone: NoticeTone }) {
  const props = { size: 16, 'aria-hidden': true } as const
  let glyph: ReactNode
  switch (icon) {
    case 'spinner':
      glyph = <LoaderCircle {...props} className={uiClasses.spin} />
      break
    case 'clock':
      glyph = <Clock {...props} />
      break
    case 'alert':
      glyph = <CircleAlert {...props} />
      break
    case 'help':
      glyph = <CircleHelp {...props} />
      break
    case 'warn':
      glyph = <TriangleAlert {...props} />
      break
    case 'ban':
      glyph = <Ban {...props} />
      break
    case 'check':
      glyph = <CircleCheck {...props} />
      break
    case 'file-x':
      glyph = <FileX {...props} />
      break
  }
  return (
    <span className="eval-ui-ad-glyph" data-tone={tone}>
      {glyph}
    </span>
  )
}

export function StateNotice({
  result,
  now,
  narrow,
  limits,
  actions,
}: {
  result: AnalysisResult
  now: number
  narrow: boolean
  limits: MonitorLimits | undefined
  actions: DetailActions
}) {
  const copy = describeState(result, now, limits)
  if (!copy) return null
  const busy = actions.busy !== null
  const size = narrow ? 'lg' : 'sm'
  const signals = result.record.counters.diagnostics

  const buttons: Record<NoticeAction, ReactNode> = {
    cancel: (
      <Button key="cancel" variant="pill" size={size} disabled={busy} onClick={actions.cancel}>
        Cancel analysis
      </Button>
    ),
    reanalyze: (
      <Button key="reanalyze" variant="pill" size={size} disabled={busy} onClick={actions.reanalyze}>
        Reanalyze
      </Button>
    ),
    signals: (
      <Button key="signals" variant="ghost" size={size} onClick={actions.viewSignals}>
        {signals === 1 ? 'View signal' : 'View signals'}
      </Button>
    ),
    session: actions.openInvestigation ? (
      <Button key="session" variant="ghost" size={size} onClick={actions.openInvestigation}>
        Open investigation session
      </Button>
    ) : null,
  }
  const shown = copy.actions.map((action) => buttons[action]).filter(Boolean)

  return (
    <div className="eval-ui-ad-notice" data-tone={copy.tone}>
      <StatusPanel
        variant={VARIANT[copy.tone]}
        role={copy.tone === 'alert' ? 'alert' : undefined}
        icon={<Glyph icon={copy.icon} tone={copy.tone} />}
        headline={copy.title}
        detail={
          <div className="eval-ui-ad-notice-body">
            <p>{copy.body}</p>
            {copy.message ? <p className="eval-ui-ad-quiet">{copy.message}</p> : null}
            {copy.detail ? <p className="eval-ui-ad-mono-quiet">{copy.detail}</p> : null}
          </div>
        }
      />
      {shown.length ? <div className="eval-ui-ad-notice-actions">{shown}</div> : null}
    </div>
  )
}
