import {
  Button,
  EmptyState,
  IconButton,
  Input,
  List,
  ListGroup,
  ListItem,
  PageSidebar,
  type PanelSide,
  SegmentedControl,
  Skeleton,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import {
  ChevronDown,
  ChevronUp,
  Pause,
  Play,
  RefreshCw,
  Settings2,
  SlidersHorizontal,
  SquareArrowOutUpRight,
  TriangleAlert,
} from 'lucide-react'
import {
  type FormEvent,
  Fragment,
  type MutableRefObject,
  type ReactNode,
  type RefObject,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { formatDate } from '../../components'
import type { Filter, Tone } from '../../model'
import type { AnalysisRecord, MonitorState } from '../../types'
import { MonitorNotices } from './Notices'
import {
  CARD_LABEL,
  type CardState,
  cappedNote,
  capRow,
  cardState,
  emptyFilterTitle,
  FILTERS,
  filterLabel,
  groupHistory,
  groupSource,
  groupTally,
  type HistoryItem,
  memberMeta,
  queueLine,
  type ReviewIndex,
  reviewLine,
  reviewMeta,
  rowCause,
  rowMeta,
  rowStatus,
  rowTime,
  rowTitle,
  SIDEBAR_WIDTH,
  skippedRow,
  type TriageProblem,
  todayLine,
  triageNotice,
  triageRow,
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
  capped: 'warn',
  observing: 'accent',
  unavailable: 'alert',
}

export interface SidebarProps {
  panelSide: PanelSide
  narrow: boolean
  state: MonitorState | null
  /** The first read of the monitor failed. */
  stateError: string | null
  /** What people decided about the suggestions; `null` until read, or when the worker predates it. */
  reviews: ReviewIndex | null
  /** Why triage cannot run, when the provider check or the last analysis says so. */
  triage: TriageProblem | undefined
  /** The provider check has answered at least once. */
  triageChecked: boolean
  triageChecking: boolean
  onCheckTriage: () => void
  /** Opens the settings at the daily cost cap. */
  onChangeCap: () => void
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
  /**
   * Rejects with the error to show under the form; resolves with whether the analysis already existed, or that the
   * person backed out of the question that comes first.
   */
  onAnalyze: (sessionId: string) => Promise<{ reused: boolean; cancelled: boolean }>
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
          options={FILTERS.map(({ value, label }) => ({
            value,
            label: filterLabel(value, label, props.records, props.reviews),
            icon: false as const,
          }))}
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
  triage,
  triageChecked,
  triageChecking,
  narrow,
  onToggleObservation,
  onToggleSettings,
  onCheckTriage,
  onChangeCap,
}: SidebarProps) {
  const card = cardState(state, stateError !== null)
  const config = state?.config ?? null
  // Observing with triage that cannot run still observes, but the dot says it will not get far.
  const tone = card === 'observing' && triage ? 'warn' : CARD_TONE[card]
  const skipped = card === 'capped' ? skippedRow(state) : null
  const cap = state && config ? capRow(state.cost) : null

  return (
    <section className="eval-ui-status" aria-label="Monitor status">
      <div className="eval-ui-status-head">
        <Dot tone={tone} pulse={card === 'observing' && !triage} />
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
      {card === 'capped' && state ? <p className="eval-ui-status-note">{cappedNote(state.cost)}</p> : null}
      {triage && card !== 'unconfigured' ? (
        <StatusPanel
          variant="warn"
          role="status"
          icon={<TriangleAlert aria-hidden size={16} />}
          headline="Triage unavailable"
          detail={
            <div className="eval-ui-status-triage">
              <p>{triageNotice(triage)}</p>
              {/* The provider check lists models: it can say a key came back, never that credits did. */}
              {triage.source === 'check' ? (
                <Button
                  variant="ghost"
                  size={narrow ? 'lg' : 'sm'}
                  disabled={triageChecking}
                  aria-busy={triageChecking || undefined}
                  onClick={onCheckTriage}
                >
                  <RefreshCw aria-hidden size={16} />
                  Check again
                </Button>
              ) : null}
            </div>
          }
        />
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
          <dd data-tone={triage ? 'alert' : undefined}>
            {triageRow(triage, triageChecked, state?.triage?.provider ?? TRIAGE_PROVIDER)}
          </dd>
          <dt>Queue</dt>
          <dd>{records ? <Parts text={queueLine(records, now, state?.limits.max_active_analyses)} /> : '—'}</dd>
          <dt>Today</dt>
          <dd>{records ? <Parts text={todayLine(records, now)} /> : '—'}</dd>
          {cap ? (
            <>
              <dt>Cap</dt>
              <dd title="The cap counts a UTC day, and the cost of analyses only: replays are shown, not counted.">
                <Parts text={cap} />
              </dd>
            </>
          ) : null}
          {skipped ? (
            <>
              <dt>Skipped</dt>
              <dd>
                <Parts text={skipped} />
              </dd>
            </>
          ) : null}
        </dl>
      ) : null}
      {card === 'capped' ? (
        <Button variant="ghost" size={narrow ? 'lg' : 'sm'} className="eval-ui-status-cap" onClick={onChangeCap}>
          <SlidersHorizontal aria-hidden size={16} />
          Change cap
        </Button>
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
      const { reused, cancelled } = await onAnalyze(sessionId)
      // Backed out: the id stays, ready to be sent after all.
      if (cancelled) return
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
  reviews,
  selectedId,
  settingsOpen,
  now,
  onSelect,
  onOpenSession,
  onRetry,
}: SidebarProps) {
  const items = records ? groupHistory(records, filter, reviews) : []
  const loading = records === null && listError === null
  // Groups the person opened; the one holding the selection opens when the selection moves into it.
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set())
  const selectedKey = settingsOpen
    ? undefined
    : records?.find((record) => record.evaluation_id === selectedId)?.observation_key
  useEffect(() => {
    if (selectedKey) setOpen((current) => (current.has(selectedKey) ? current : new Set(current).add(selectedKey)))
  }, [selectedKey])
  const toggle = (key: string) =>
    setOpen((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  const row = (record: AnalysisRecord, grouped: boolean) => (
    <Row
      key={record.evaluation_id}
      record={record}
      grouped={grouped}
      reviews={reviews}
      now={now}
      // Settings are open in the main pane: no analysis is the current one.
      selected={!settingsOpen && record.evaluation_id === selectedId}
      onSelect={onSelect}
      onOpenSession={onOpenSession}
    />
  )

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
      {records && records.length > 0 && items.length === 0 ? (
        <EmptyState
          compact
          title={emptyFilterTitle(filter, reviews !== null)}
          description="Nothing in the list matches this filter."
          action={{ label: 'Show all', onClick: () => onFilter('all') }}
        />
      ) : null}
      {items.length > 0 ? (
        <List>
          {items.map((item: HistoryItem) =>
            item.kind === 'row' ? (
              row(item.record, false)
            ) : (
              <TurnGroup
                key={item.key}
                item={item}
                open={open.has(item.key)}
                onToggle={() => toggle(item.key)}
                renderRow={(record) => row(record, true)}
              />
            ),
          )}
        </List>
      ) : null}
    </nav>
  )
}

/**
 * The analyses of one observed turn: the header says which turn and what happened to it, the newest shown analysis
 * stays visible and the rest is one click away.
 */
function TurnGroup({
  item,
  open,
  onToggle,
  renderRow,
}: {
  item: Extract<HistoryItem, { kind: 'group' }>
  open: boolean
  onToggle: () => void
  renderRow: (record: AnalysisRecord) => ReactNode
}) {
  const bodyId = useId()
  const [newest, ...earlier] = item.shown
  const head = item.members[0]
  const Chevron = open ? ChevronUp : ChevronDown
  return (
    <ListGroup className="eval-ui-group">
      <button
        type="button"
        className="eval-ui-group-head"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={onToggle}
      >
        <span className="eval-ui-group-title">
          <Chevron size={16} aria-hidden="true" />
          <span className="eval-ui-group-name" title={rowTitle(head)}>
            {rowTitle(head)}
          </span>
        </span>
        <span className="eval-ui-group-source">{groupSource(head)}</span>
        <span className="eval-ui-group-tally">{groupTally(item.members)}</span>
      </button>
      <div id={bodyId} className="eval-ui-group-rows">
        {renderRow(newest)}
        {open ? earlier.map(renderRow) : null}
        {!open && earlier.length > 0 ? (
          <button type="button" className="eval-ui-group-more" onClick={onToggle}>
            <ChevronDown size={16} aria-hidden="true" />
            Show {earlier.length} earlier {earlier.length === 1 ? 'analysis' : 'analyses'}
          </button>
        ) : null}
      </div>
    </ListGroup>
  )
}

/**
 * A row selects the analysis; the icon beside it opens the observed session
 * without selecting. Two sibling buttons: a button cannot hold a button.
 * Inside a group the turn is in the header, so a row says what became of this
 * analysis (the cause of a failure, what it cost and took) instead of the title.
 */
function Row({
  record,
  grouped,
  reviews,
  now,
  selected,
  onSelect,
  onOpenSession,
}: {
  record: AnalysisRecord
  grouped: boolean
  reviews: ReviewIndex | null
  now: number
  selected: boolean
  onSelect: (evaluationId: string) => void
  onOpenSession?: (sessionId: string) => void
}) {
  const status = rowStatus(record)
  const cause = rowCause(record)
  // A finished analysis with suggestions says where they stand; the rest keeps the monitor's own words.
  const label = reviewLine(record, reviews) ?? status.label
  const standing = reviewMeta(record, reviews)
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
              {label}
            </span>
            <time
              className="eval-ui-row-time"
              dateTime={new Date(record.created_at).toISOString()}
              title={formatDate(record.created_at)}
            >
              {rowTime(record, now)}
            </time>
          </span>
          {grouped ? (
            <>
              {cause ? (
                <span className="eval-ui-row-title" title={cause}>
                  {cause}
                </span>
              ) : null}
              {standing ? <span className="eval-ui-row-meta">{standing}</span> : null}
              <span className="eval-ui-row-meta">{memberMeta(record)}</span>
            </>
          ) : (
            <>
              <span className="eval-ui-row-title" title={rowTitle(record)}>
                {rowTitle(record)}
              </span>
              {standing ? <span className="eval-ui-row-meta">{standing}</span> : null}
              <span className="eval-ui-row-meta">
                {record.session_id} · {rowMeta(record)}
              </span>
            </>
          )}
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
