// The attach dialog's plan hint, the Fill with Jev row, and what Jev's answer
// (or the run list) leaves on screen: notices, the quick-pick pairs. Copy and
// numbers come from `jev-fill.ts`; this file only lays them out.
import { Button, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { CircleAlert, ExternalLink, Info, LoaderCircle, Pencil, Plug, Sparkles, TriangleAlert } from 'lucide-react'
import { type ReactNode, useEffect, useId, useRef } from 'react'
import {
  editedNotice,
  type JevFill,
  noFitNotice,
  noPairNotice,
  noRunsNotice,
  type PairlessNotice,
  type Proposed,
  planHint,
  proposedNotice,
  quickPicks,
} from './jev-fill'
import { Inline } from './marks'

/** The plan's line above the pickers, with the way out to the E2E page. */
export function PlanHint({
  scenarioId,
  promoted,
  disabled,
  narrow,
  onOpenE2e,
}: {
  scenarioId: string | null
  /** Running the case in E2E is the next step: Open E2E is a button, not a quiet link. */
  promoted: boolean
  /** Leaving the dialog is refused while a save is in flight. */
  disabled: boolean
  narrow: boolean
  /** Hidden when the console cannot open the E2E page. */
  onOpenE2e: (() => void) | undefined
}) {
  return (
    <div className="eval-ui-val-hint">
      <p className="eval-ui-val-hint-text">
        <Inline text={planHint(scenarioId)} />
      </p>
      {onOpenE2e ? (
        <Button
          type="button"
          variant={promoted ? 'pill' : 'ghost'}
          size={narrow ? 'lg' : 'sm'}
          disabled={disabled}
          onClick={onOpenE2e}
        >
          Open E2E
          <ExternalLink className={uiClasses.icon} aria-hidden />
        </Button>
      ) : null}
    </div>
  )
}

export function FillRow({
  asking,
  replaces,
  disabled,
  narrow,
  onFill,
}: {
  asking: boolean
  /** A picker already holds a run: Jev's pair will take its place. */
  replaces: boolean
  disabled: boolean
  narrow: boolean
  onFill: () => void
}) {
  return (
    <div className="eval-ui-val-fill">
      {/* While asking the button stays focusable (aria-disabled, not disabled): a keyboard user keeps their place for up to 90 s. */}
      <Button
        type="button"
        variant="pill"
        size={narrow ? 'lg' : 'sm'}
        disabled={disabled}
        aria-disabled={asking || undefined}
        aria-busy={asking || undefined}
        onClick={asking ? undefined : onFill}
      >
        {asking ? (
          <>
            <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
            Asking Jev…
          </>
        ) : (
          <>
            <Sparkles className={uiClasses.icon} aria-hidden />
            Fill with Jev
          </>
        )}
      </Button>
      <p className="eval-ui-val-quiet eval-ui-val-small eval-ui-val-fill-note">
        {replaces
          ? 'Replaces both runs below with the pair Jev picks. You confirm before anything is attached.'
          : 'Jev reads run names, Harness builds, model and scenario to pick a pair. You confirm before anything is attached.'}
      </p>
      <span className="eval-ui-val-sr" role="status">
        {asking ? 'Asking Jev…' : ''}
      </span>
    </div>
  )
}

/** The lines of a notice's detail: the sentence first, quiet lines after. */
function Lines({ children }: { children: ReactNode }) {
  return <div className="eval-ui-val-pds">{children}</div>
}

const Quiet = ({ children }: { children: ReactNode }) => (
  <p className="eval-ui-val-pd" data-quiet>
    {children}
  </p>
)

function PairlessPanel({ notice, icon }: { notice: PairlessNotice; icon: ReactNode }) {
  return (
    <StatusPanel
      variant="info"
      role="status"
      icon={icon}
      headline={notice.headline}
      detail={
        <Lines>
          <p className="eval-ui-val-pd">{notice.detail}</p>
          <Quiet>{notice.foot}</Quiet>
        </Lines>
      }
    />
  )
}

function ProposedPanel({ fill, scenarioId }: { fill: Proposed; scenarioId: string | null }) {
  const notice = proposedNotice(fill, scenarioId)
  const warn = notice.tone === 'warn'
  return (
    <StatusPanel
      variant={warn ? 'warn' : 'info'}
      role="status"
      icon={
        warn ? (
          <TriangleAlert className={uiClasses.icon} aria-hidden />
        ) : (
          <Sparkles className={uiClasses.icon} aria-hidden />
        )
      }
      headline={notice.headline}
      detail={
        <Lines>
          <p className="eval-ui-val-pd">{notice.detail}</p>
          {notice.advice.map((line) => (
            <p key={line} className="eval-ui-val-pd">
              {line}
            </p>
          ))}
          <Quiet>{notice.foot}</Quiet>
          {notice.stack.head ? (
            <Quiet>
              {notice.stack.head}
              {notice.stack.mono ? (
                <>
                  {' '}
                  <span className="eval-ui-val-mono">{notice.stack.mono}</span>
                </>
              ) : null}
            </Quiet>
          ) : null}
        </Lines>
      }
    />
  )
}

/** Once a picker Jev filled was changed, the pair is no longer Jev's: one quiet line says so. */
function EditedLine({ headline, detail }: { headline: string; detail: string }) {
  return (
    <div className="eval-ui-val-edited" role="status">
      <Pencil className={uiClasses.icon} aria-hidden />
      <span className="eval-ui-val-edited-text">
        <span className="eval-ui-val-label">{headline}</span>
        <span className="eval-ui-val-quiet eval-ui-val-small">{detail}</span>
      </span>
    </div>
  )
}

/** Jev's other pairs, one tap each: both pickers take the pair. */
function QuickPicks({
  fill,
  nameOf,
  disabled,
  onChoose,
}: {
  fill: Proposed
  nameOf: (executionId: string) => string
  disabled: boolean
  onChoose: (index: number) => void
}) {
  const headingId = useId()
  const picks = quickPicks(fill)
  if (picks.length === 0) return null
  return (
    <div className="eval-ui-val-alts">
      <span id={headingId} className="eval-ui-val-label">
        Other pairs Jev considered
      </span>
      <ul className="eval-ui-val-alt-list" aria-labelledby={headingId}>
        {picks.map((pick) => (
          <li key={pick.index}>
            <button
              type="button"
              className="eval-ui-val-alt"
              data-pick={pick.index}
              disabled={disabled}
              onClick={() => onChoose(pick.index)}
            >
              <span className="eval-ui-val-alt-pair">
                <span className="eval-ui-val-alt-who">Baseline</span>
                <span>{nameOf(pick.pair.baseline)}</span>
              </span>
              <span className="eval-ui-val-alt-pair">
                <span className="eval-ui-val-alt-who">Candidate</span>
                <span>{nameOf(pick.pair.candidate)}</span>
              </span>
              <span className="eval-ui-val-quiet eval-ui-val-small">{pick.quiet}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** Everything Jev's answer leaves under the pickers. */
export function JevNotices({
  fill,
  scenarioId,
  nameOf,
  disabled,
  onChoose,
}: {
  fill: JevFill
  scenarioId: string | null
  nameOf: (executionId: string) => string
  disabled: boolean
  onChoose: (index: number) => void
}) {
  const zone = useRef<HTMLDivElement>(null)
  const notice = useRef<HTMLDivElement>(null)
  const back = useRef<number | null>(null)
  // One answer, one scroll: switching pairs or editing keeps `pairs`, a new answer replaces it.
  const answer = fill.kind === 'proposed' ? fill.pairs : fill
  useEffect(() => {
    notice.current?.scrollIntoView?.({ block: 'nearest' })
  }, [answer])
  // The pair the user chose leaves the list: the focus goes to the row of the pair it replaced, else to the notice.
  useEffect(() => {
    if (back.current === null) return
    const replaced = back.current
    back.current = null
    const root = zone.current
    const row =
      root?.querySelector<HTMLElement>(`[data-pick="${replaced}"]`) ??
      root?.querySelector<HTMLElement>('.eval-ui-val-alt')
    ;(row ?? notice.current)?.focus()
  })

  const first = ((): ReactNode => {
    switch (fill.kind) {
      case 'proposed': {
        const edited = editedNotice(fill)
        return edited ? <EditedLine {...edited} /> : <ProposedPanel fill={fill} scenarioId={scenarioId} />
      }
      case 'none_fits':
        return (
          <PairlessPanel notice={noFitNotice(fill.tally)} icon={<Sparkles className={uiClasses.icon} aria-hidden />} />
        )
      case 'no_comparable_pair':
        return (
          <PairlessPanel
            notice={noPairNotice(fill.tally, scenarioId)}
            icon={<Info className={uiClasses.icon} aria-hidden />}
          />
        )
      case 'failed':
        return (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<CircleAlert className={uiClasses.icon} aria-hidden />}
            headline="Couldn't ask Jev"
            detail={
              <Lines>
                <p className="eval-ui-val-pd">
                  <span className="eval-ui-val-mono">{fill.message}</span>
                </p>
                <Quiet>Nothing was filled. You can still choose the runs yourself.</Quiet>
              </Lines>
            }
          />
        )
      default:
        return null
    }
  })()
  if (!first) return null
  return (
    <div ref={zone} className="eval-ui-val-zone">
      <div ref={notice} className="eval-ui-val-notice" tabIndex={-1}>
        {first}
      </div>
      {fill.kind === 'proposed' ? (
        <QuickPicks
          fill={fill}
          nameOf={nameOf}
          disabled={disabled}
          onChoose={(index) => {
            back.current = fill.chosen
            onChoose(index)
          }}
        />
      ) : null}
    </div>
  )
}

/** The E2E service holds no runs at all (state 9a). */
export function NoRunsPanel({ scenarioId }: { scenarioId: string | null }) {
  const notice = noRunsNotice(scenarioId)
  return (
    <StatusPanel
      variant="info"
      role="status"
      icon={<Info className={uiClasses.icon} aria-hidden />}
      headline={notice.headline}
      detail={notice.detail}
    />
  )
}

/** The run list did not load (state 9b); Try again is the dialog's primary action. */
export function RunsDownPanel() {
  return (
    <StatusPanel
      variant="warn"
      role="alert"
      icon={<Plug className={uiClasses.icon} aria-hidden />}
      headline="E2E service unavailable"
      detail="The run list needs the E2E worker, and it isn't answering. There is nothing to choose from or to send to Jev, and nothing was saved."
    />
  )
}
