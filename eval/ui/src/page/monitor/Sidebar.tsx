import {
  Button,
  EmptyState,
  IconButton,
  Input,
  List,
  ListItem,
  PageSidebar,
  type PanelSide,
  SegmentedControl,
  Skeleton,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Pause, Play, Settings2, SquareArrowOutUpRight } from 'lucide-react'
import {
  type FormEvent,
  Fragment,
  type MutableRefObject,
  type RefObject,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { formatDate } from '../../components'
import { type Filter, matchesFilter, type Tone } from '../../model'
import type { AnalysisRecord, MonitorState } from '../../types'
import { MonitorNotices } from './Notices'
import {
  CARD_LABEL,
  type CardState,
  cardState,
  clock,
  emptyFilterTitle,
  FILTERS,
  queueLine,
  rowMeta,
  rowStatus,
  rowTitle,
  SIDEBAR_WIDTH,
  todayLine,
} from './shell-state'

/** The triage provider the monitor always uses (shown until the API reports one). */
const TRIAGE_PROVIDER = 'judge-typesafe'
const SKELETON_ROWS = 4

type DotTone = 'accent' | 'alert' | 'warn' | 'ok' | 'ink'

const DOT_TONE: Record<Tone, DotTone> = { ok: 'ok', accent: 'accent', warn: 'warn', alert: 'alert', neutral: 'ink' }

function Dot({ tone, pulse }: { tone: Tone; pulse?: boolean }) {
  return (
    <StatusDot tone={DOT_TONE[tone]} pulse={pulse} className={tone === 'neutral' ? 'eval-ui-dot-ghost' : undefined} />
  )
}

const CARD_TONE: Record<CardState, Tone> = {
  loading: 'neutral',
  unconfigured: 'neutral',
  paused: 'neutral',
  observing: 'accent',
  unavailable: 'alert',
}

export interface SidebarProps {
  panelSide: PanelSide
  narrow: boolean
  state: MonitorState | null
  /** The first read of the monitor failed. */
  stateError: string | null
  records: AnalysisRecord[] | null
  listError: string | null
  now: number
  filter: Filter
  onFilter: (filter: Filter) => void
  selectedId: string | null
  settingsOpen: boolean
  togglePending: boolean
  actionError: string | null
  onToggleObservation: () => void
  onToggleSettings: () => void
  onSelect: (evaluationId: string) => void
  /** Rejects with the error to show under the form; resolves with whether the analysis already existed. */
  onAnalyze: (sessionId: string) => Promise<{ reused: boolean }>
  /** Shows the observed session of a row; absent when the console cannot open a conversation. */
  onOpenSession?: (sessionId: string) => void
  onRetry: () => void
  inputRef: RefObject<HTMLInputElement | null>
  /** The list's scroll offset, kept while the detail covers it. */
  scrollMemo: MutableRefObject<number>
  /** Focus the selected row when the list comes back from the detail. */
  restoreFocus: boolean
}

export function Sidebar(props: SidebarProps) {
  const { panelSide, narrow, scrollMemo, restoreFocus } = props
  const scroller = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = scrollMemo.current
  }, [scrollMemo])
  useEffect(() => {
    if (restoreFocus) {
      scroller.current?.querySelector<HTMLElement>('[aria-current="true"]')?.focus({ preventScroll: true })
    }
    // Only when the list mounts: later renders must not steal focus.
  }, [])

  return (
    <PageSidebar
      aria-label="Monitor and analysis history"
      side={panelSide}
      width={SIDEBAR_WIDTH}
      narrow={narrow}
      className="eval-ui-sidebar"
    >
      <div
        ref={scroller}
        className="eval-ui-side"
        onScroll={(event) => {
          scrollMemo.current = event.currentTarget.scrollTop
        }}
      >
        <StatusCard {...props} />
        <MonitorNotices
          state={props.state}
          loadError={props.stateError}
          records={props.records}
          now={props.now}
          narrow={narrow}
          onRetry={props.onRetry}
        />
        {props.actionError ? <StatusPanel variant="alert" role="alert" headline={props.actionError} /> : null}
        <AnalyzeForm {...props} />
        <SegmentedControl
          variant="radio"
          aria-label="Filter analyses"
          className="eval-ui-filter"
          itemClassName="eval-ui-filter-item"
          value={props.filter}
          onChange={props.onFilter}
          options={FILTERS.map(({ value, label }) => ({ value, label, icon: false as const }))}
        />
        <History {...props} />
      </div>
    </PageSidebar>
  )
}

function StatusCard({
  state,
  stateError,
  records,
  now,
  settingsOpen,
  togglePending,
  onToggleObservation,
  onToggleSettings,
}: SidebarProps) {
  const card = cardState(state, stateError !== null)
  const config = state?.config ?? null
  const tone = CARD_TONE[card]

  return (
    <section className="eval-ui-status" aria-label="Monitor status">
      <div className="eval-ui-status-head">
        <Dot tone={tone} pulse={card === 'observing'} />
        {card === 'loading' ? (
          <Skeleton className="eval-ui-sk-label" />
        ) : (
          <span className="eval-ui-status-label">{CARD_LABEL[card]}</span>
        )}
        {config ? (
          <span className="eval-ui-status-actions">
            <IconButton
              label={config.enabled ? 'Pause observation' : 'Resume observation'}
              disabled={togglePending}
              onClick={onToggleObservation}
            >
              {config.enabled ? <Pause size={16} aria-hidden /> : <Play size={16} aria-hidden />}
            </IconButton>
            <IconButton label="Monitor settings" aria-pressed={settingsOpen} onClick={onToggleSettings}>
              <Settings2 size={16} aria-hidden />
            </IconButton>
          </span>
        ) : null}
      </div>
      {card === 'unconfigured' ? (
        <p className="eval-ui-status-note">
          Nothing is observed or sent to a model until you choose an analyst model and turn observation on.
        </p>
      ) : null}
      {card === 'loading' ? (
        <div className="eval-ui-status-skeleton" aria-busy="true">
          <Skeleton className="eval-ui-sk-line" />
          <Skeleton className="eval-ui-sk-line" />
        </div>
      ) : null}
      {config ? (
        <dl className="eval-ui-status-lines">
          <dt>Analyst</dt>
          <dd data-truncate="" title={`${config.model.model} · ${config.model.provider}`}>
            {config.model.model} · {config.model.provider}
          </dd>
          <dt>Triage</dt>
          <dd>{state?.triage?.provider ?? TRIAGE_PROVIDER}</dd>
          <dt>Queue</dt>
          <dd>{records ? <Parts text={queueLine(records, now, state?.limits.max_active_analyses)} /> : '—'}</dd>
          <dt>Today</dt>
          <dd>{records ? <Parts text={todayLine(records, now)} /> : '—'}</dd>
        </dl>
      ) : null}
    </section>
  )
}

/** A line of `·`-separated facts that wraps between facts, never inside one. */
function Parts({ text }: { text: string }) {
  return (
    <>
      {text.split(' · ').map((part, index) => (
        <Fragment key={index}>
          {index > 0 ? ' · ' : null}
          <span className="eval-ui-nowrap">{part}</span>
        </Fragment>
      ))}
    </>
  )
}

function AnalyzeForm({ state, narrow, inputRef, onAnalyze }: SidebarProps) {
  const inputId = useId()
  const hintId = useId()
  const [value, setValue] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const configured = Boolean(state?.config)
  // Until the monitor answers there is nothing to say about a model.
  const needsModel = state !== null && !configured
  const sessionId = value.trim()

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (busy || !configured) return
    if (!sessionId) {
      inputRef.current?.focus()
      return
    }
    setBusy(true)
    setError(null)
    setNote(null)
    try {
      const { reused } = await onAnalyze(sessionId)
      setValue('')
      if (reused) setNote('Already analyzed — showing the existing analysis')
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="eval-ui-analyze" aria-label="Analyze a session" onSubmit={(event) => void submit(event)}>
      <label className="eval-ui-label" htmlFor={inputId}>
        Analyze a session
      </label>
      <div className="eval-ui-analyze-row">
        <Input
          ref={inputRef}
          id={inputId}
          className="eval-ui-mono-input"
          placeholder="Session ID"
          autoComplete="off"
          spellCheck={false}
          value={value}
          disabled={!configured}
          aria-describedby={needsModel ? hintId : undefined}
          onChange={(next) => {
            setValue(next)
            setError(null)
            setNote(null)
          }}
        />
        <Button
          type="submit"
          variant="primary"
          size={narrow ? 'lg' : 'sm'}
          disabled={!configured || busy}
          aria-busy={busy || undefined}
        >
          Analyze
        </Button>
      </div>
      {needsModel ? (
        <span id={hintId} className="eval-ui-hint">
          Needs an analyst model first.
        </span>
      ) : null}
      {error ? (
        <p className="eval-ui-form-error" role="alert">
          {error}
        </p>
      ) : null}
      {note ? (
        <p className="eval-ui-hint" role="status">
          {note}
        </p>
      ) : null}
    </form>
  )
}

function History({
  narrow,
  records,
  listError,
  filter,
  onFilter,
  selectedId,
  settingsOpen,
  onSelect,
  onOpenSession,
  onRetry,
}: SidebarProps) {
  const rows = records?.filter((record) => matchesFilter(record, filter)) ?? []
  const loading = records === null && listError === null

  return (
    <nav className="eval-ui-history" aria-label="Analyses" aria-busy={loading || undefined}>
      {listError ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="Couldn't load analyses"
          detail={
            <>
              The eval worker didn't answer. Analyses already running aren't affected.{' '}
              <code className="eval-ui-mono">{listError}</code>
            </>
          }
          action={
            <Button variant="pill" size={narrow ? 'lg' : 'sm'} onClick={onRetry}>
              Retry
            </Button>
          }
        />
      ) : null}
      {loading ? <RowSkeletons /> : null}
      {records && records.length === 0 && !listError ? (
        <EmptyState
          compact
          title="No analyses yet"
          description="Finished sessions show up here once the monitor is on."
        />
      ) : null}
      {records && records.length > 0 && rows.length === 0 ? (
        <EmptyState
          compact
          title={emptyFilterTitle(filter)}
          description="Nothing in the list matches this filter."
          action={{ label: 'Show all', onClick: () => onFilter('all') }}
        />
      ) : null}
      {rows.length > 0 ? (
        <List>
          {rows.map((record) => (
            <Row
              key={record.evaluation_id}
              record={record}
              // Settings are open in the main pane: no analysis is the current one.
              selected={!settingsOpen && record.evaluation_id === selectedId}
              onSelect={onSelect}
              onOpenSession={onOpenSession}
            />
          ))}
        </List>
      ) : null}
    </nav>
  )
}

/**
 * A row selects the analysis; the icon beside it opens the observed session
 * without selecting. Two sibling buttons: a button cannot hold a button.
 */
function Row({
  record,
  selected,
  onSelect,
  onOpenSession,
}: {
  record: AnalysisRecord
  selected: boolean
  onSelect: (evaluationId: string) => void
  onOpenSession?: (sessionId: string) => void
}) {
  const status = rowStatus(record)
  return (
    <div className="eval-ui-row-wrap">
      <ListItem
        className="eval-ui-row"
        selected={selected}
        aria-current={selected ? 'true' : undefined}
        onClick={() => onSelect(record.evaluation_id)}
      >
        <span className="eval-ui-row-body">
          <span className="eval-ui-row-top">
            <Dot tone={status.tone} />
            <span className="eval-ui-row-status" data-tone={status.quiet ? 'neutral' : status.tone}>
              {status.label}
            </span>
            <time
              className="eval-ui-row-time"
              dateTime={new Date(record.created_at).toISOString()}
              title={formatDate(record.created_at)}
            >
              {clock(record.created_at)}
            </time>
          </span>
          <span className="eval-ui-row-title" title={rowTitle(record)}>
            {rowTitle(record)}
          </span>
          <span className="eval-ui-row-meta">
            {record.session_id} · {rowMeta(record)}
          </span>
        </span>
      </ListItem>
      {onOpenSession ? (
        <IconButton
          label={`Open session ${record.session_id}`}
          tooltip="Open session"
          className="eval-ui-row-open"
          onClick={() => onOpenSession(record.session_id)}
        >
          <SquareArrowOutUpRight size={16} aria-hidden />
        </IconButton>
      ) : null}
    </div>
  )
}

function RowSkeletons() {
  return (
    <div className="eval-ui-skeleton-rows" aria-hidden="true">
      {Array.from({ length: SKELETON_ROWS }, (_, index) => (
        <div className="eval-ui-skeleton-row" key={index}>
          <div className="eval-ui-skeleton-top">
            <Skeleton className="eval-ui-sk-dot" />
            <Skeleton className="eval-ui-sk-label" />
            <Skeleton className="eval-ui-sk-time" />
          </div>
          <Skeleton className="eval-ui-sk-title" />
          <Skeleton className="eval-ui-sk-meta" />
        </div>
      ))}
    </div>
  )
}
