/**
 * Ad-hoc SQL: the shared Monaco `CodeEditor` (⌘⏎ runs), an explain action,
 * per-database history in localStorage, and results in the shared grid with
 * duration. Reads run through `database::query`, writes through
 * `database::execute` — a "write" note appears by the actions before a
 * mutating statement runs, and the result line reports affected rows.
 *
 * A buffer holding several statements runs them one after another, stopping
 * at the first failure, with a step per statement (its own result, rows and
 * time); a selection runs on its own from the editor's selection bar. Reads
 * are capped by the row limit (`max_rows`, default 100) and say so when the
 * cap cut them short. Format goes through `database::formatSql`.
 *
 * Explain goes through `database::explain`, which parses the dialect's own
 * plan format into one tree. `driver` only tells the statement splitter
 * where strings and comments end.
 */

import {
  Button,
  CodeEditor,
  type CodeEditorHandle,
  type CodeEditorSelectionAction,
  type Host,
  IconButton,
  Kbd,
  ShadcnSelect,
  ShadcnSelectContent,
  ShadcnSelectItem,
  ShadcnSelectTrigger,
  ShadcnSelectValue,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { CircleAlert, History, LoaderCircle, Play, SquareTerminal, WandSparkles, X } from 'lucide-react'
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from 'react'
import { errText } from '../lib/errors'
import { type ExplainResult, explain, formatSql } from '../lib/rpc'
import { splitStatements } from '../lib/sql'
import { type AdhocResult, type DbDriver, ddlInfo, isReadOnlySql, resultSummary, runAdhocSql } from './db-data'
import { PlanTree } from './PlanTree'
import { ResultGrid } from './result-grid'
import { WriteOutcome } from './WriteOutcome'

interface SqlPanelProps {
  host: Host
  db: string
  /** For the statement splitter: `#` comments on MySQL, `$$` bodies on Postgres. */
  driver?: DbDriver
  /** `database::formatSql` is registered. */
  canFormat?: boolean
  /** The worker honours `max_rows` (and reports `truncated`). */
  canLimit?: boolean
  /** Prefill from "query this table" affordances; applied when it changes. */
  seedSql?: string
  /** Table names in the active database — fed to the editor's autocomplete
      alongside the SQL keywords. */
  tables?: readonly string[]
  /**
   * A statement that actually runs against *this* database, offered as the
   * first thing to try. Quoted by the caller, which is the only place that
   * knows the driver. Absent when the database has no tables.
   */
  starterSql?: string
  /**
   * Called after a write commits, so the page can refresh schema-derived
   * state — otherwise the table list keeps showing a dropped table and
   * disconfirms what just happened.
   */
  onWrite?: () => void
  /**
   * Reports whether the editor holds a statement worth guarding — the page
   * asks before a database switch remounts (and so empties) this panel.
   * Seeded text is recreatable in one click, so only a statement the user
   * typed or edited counts.
   */
  onDirtyChange?: (dirty: boolean) => void
  /** Show a table's rows in the Data panel — offered after a write to it. */
  onOpenTable?: (table: string) => void
}

/** Imperative handle so the page's `S` command can move focus into the
    editor once it switches the panel to SQL. */
export interface SqlPanelHandle {
  focus(): void
}

const HISTORY_LIMIT = 20

/** Row caps offered for reads; `0` is no cap. */
const ROW_LIMITS = [100, 500, 1000, 5000, 0] as const
const DEFAULT_ROW_LIMIT = 100
const LIMIT_KEY = 'iii-console:database:sql-row-limit'
const SPLIT_KEY = 'iii-console:database:sql-editor-height'
/** The editor never shrinks below a few lines, nor squeezes the results out. */
const MIN_EDITOR_PX = 120
const MIN_RESULTS_PX = 160
const SPLIT_STEP_PX = 24

function loadSplit(): number | null {
  try {
    const raw = Number(window.localStorage.getItem(SPLIT_KEY))
    return Number.isFinite(raw) && raw >= MIN_EDITOR_PX ? raw : null
  } catch {
    return null
  }
}

function saveSplit(px: number | null) {
  try {
    if (px === null) window.localStorage.removeItem(SPLIT_KEY)
    else window.localStorage.setItem(SPLIT_KEY, String(Math.round(px)))
  } catch {
    // a remembered split is a convenience
  }
}

function loadLimit(): number {
  try {
    const raw = Number(window.localStorage.getItem(LIMIT_KEY))
    return (ROW_LIMITS as readonly number[]).includes(raw) && window.localStorage.getItem(LIMIT_KEY) !== null
      ? raw
      : DEFAULT_ROW_LIMIT
  } catch {
    return DEFAULT_ROW_LIMIT
  }
}

function saveLimit(limit: number) {
  try {
    window.localStorage.setItem(LIMIT_KEY, String(limit))
  } catch {
    // a remembered limit is a convenience
  }
}

/** One statement of a run, with what it returned or why it failed. */
interface RunStep {
  sql: string
  result?: AdhocResult
  /** When it finished (ms since epoch) — a write's "saved at". */
  at?: number
  error?: string
  /** Not run: an earlier statement failed. */
  skipped?: boolean
}

/** Whitespace-insensitive form, for "does the message already quote it". */
const squash = (text: string) => text.replace(/\s+/g, ' ').trim()

/** The run chord, written the way this keyboard has it. */
const RUN_CHORD =
  typeof navigator !== 'undefined' && /mac/i.test(navigator.platform || navigator.userAgent) ? '⌘⏎' : 'ctrl+⏎'

/** The keyword slice offered as-you-type; the table names ride alongside. */
const SQL_KEYWORDS = [
  'select',
  'from',
  'where',
  'order by',
  'group by',
  'having',
  'limit',
  'offset',
  'join',
  'left join',
  'inner join',
  'on',
  'as',
  'and',
  'or',
  'not',
  'null',
  'is null',
  'is not null',
  'in',
  'like',
  'between',
  'distinct',
  'count',
  'sum',
  'avg',
  'min',
  'max',
  'asc',
  'desc',
]

function historyKey(db: string): string {
  return `iii-console:database:sql-history:${db}`
}

function loadHistory(db: string): string[] {
  try {
    const raw = window.localStorage.getItem(historyKey(db))
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return Array.isArray(parsed) ? parsed.filter((s): s is string => typeof s === 'string') : []
  } catch {
    return []
  }
}

function saveHistory(db: string, entries: string[]) {
  try {
    window.localStorage.setItem(historyKey(db), JSON.stringify(entries.slice(0, HISTORY_LIMIT)))
  } catch {
    // storage full/unavailable — history is a convenience, not state
  }
}

export const SqlPanel = forwardRef<SqlPanelHandle, SqlPanelProps>(function SqlPanel(
  { host, db, driver, canFormat, canLimit, seedSql, tables, starterSql, onWrite, onDirtyChange, onOpenTable },
  ref,
) {
  const [sql, setSql] = useState(seedSql ?? '')
  const completions = useMemo(() => Array.from(new Set([...(tables ?? []), ...SQL_KEYWORDS])), [tables])
  const [running, setRunning] = useState(false)
  const [formatting, setFormatting] = useState(false)
  const [formatError, setFormatError] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [steps, setSteps] = useState<RunStep[] | null>(null)
  const [stepIndex, setStepIndex] = useState(0)
  const [limit, setLimit] = useState<number>(() => loadLimit())
  const statements = useMemo(() => splitStatements(sql, driver ?? 'unknown'), [sql, driver])
  const step = steps?.[stepIndex] ?? null
  const outcome = step?.result ?? null
  const [history, setHistory] = useState<string[]>(() => loadHistory(db))
  const [historyOpen, setHistoryOpen] = useState(false)
  const [plan, setPlan] = useState<ExplainResult | null>(null)
  // The statement the current outcome belongs to — once the editor moves on,
  // the result line dims rather than passing off old numbers as current.
  const [ranSql, setRanSql] = useState<string | null>(null)
  // Whole seconds since the run started: a slow query should read as
  // working, not hung.
  const [elapsed, setElapsed] = useState(0)
  useEffect(() => {
    if (!running) return
    setElapsed(0)
    const started = Date.now()
    const timer = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 500)
    return () => clearInterval(timer)
  }, [running])
  // Editor height in px once dragged; null = the default share of the pane.
  const [editorHeight, setEditorHeight] = useState<number | null>(() => loadSplit())
  const panelRef = useRef<HTMLDivElement>(null)
  const topRef = useRef<HTMLDivElement>(null)
  const editorRef = useRef<CodeEditorHandle>(null)
  const actionsRef = useRef<HTMLDivElement>(null)
  const historyRef = useRef<HTMLDivElement>(null)
  const historyBtnRef = useRef<HTMLSpanElement>(null)
  useImperativeHandle(ref, () => ({ focus: () => editorRef.current?.focus() }), [])

  // Dropdown manners: Escape and clicking elsewhere both dismiss. The
  // trigger is excluded from "elsewhere" so its own toggle doesn't
  // close-then-reopen in one click.
  useEffect(() => {
    if (!historyOpen) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setHistoryOpen(false)
    }
    const onDown = (e: MouseEvent) => {
      const t = e.target
      if (!(t instanceof Node)) return
      if (historyRef.current?.contains(t) || historyBtnRef.current?.contains(t)) return
      setHistoryOpen(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onDown)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown)
    }
  }, [historyOpen])

  // The last seeded statement, so dirtiness means "the user's own text" —
  // a seeded prefill must not trip the discard guard.
  const seededRef = useRef(seedSql ?? '')
  useEffect(() => {
    if (seedSql) {
      setSql(seedSql)
      seededRef.current = seedSql
    }
  }, [seedSql])

  const onDirtyChangeRef = useRef(onDirtyChange)
  onDirtyChangeRef.current = onDirtyChange
  useEffect(() => {
    onDirtyChangeRef.current?.(sql.trim() !== '' && sql !== seededRef.current)
  }, [sql])
  // Unmount (db switch, page close) always releases the guard.
  useEffect(() => () => onDirtyChangeRef.current?.(false), [])

  /**
   * Run `text`: one statement as before, several one after another. A
   * failure stops the run there — later statements may depend on it — and
   * the steps so far stay inspectable.
   */
  const run = useCallback(
    async (text: string) => {
      const trimmed = text.trim()
      if (!trimmed || running) return
      const parts = splitStatements(trimmed, driver ?? 'unknown')
      const queue = parts.length > 1 ? parts.map((p) => p.text) : [trimmed]
      setRunning(true)
      setError(null)
      setPlan(null)
      const done: RunStep[] = []
      let wrote = false
      const maxRows = canLimit && limit > 0 ? limit : null
      for (const statement of queue) {
        if (done.some((d) => d.error !== undefined)) {
          done.push({ sql: statement, skipped: true })
          continue
        }
        try {
          const result = await runAdhocSql(host, db, statement, { maxRows })
          if (result.write) wrote = true
          done.push({ sql: statement, result, at: Date.now() })
        } catch (err) {
          done.push({ sql: statement, error: errText(err) })
        }
      }
      const failed = done.findIndex((d) => d.error !== undefined)
      setSteps(done)
      setStepIndex(failed >= 0 ? failed : done.length - 1)
      setError(
        failed >= 0
          ? queue.length > 1
            ? `Statement ${failed + 1} of ${queue.length} failed — ${done[failed].error}`
            : (done[failed].error ?? null)
          : null,
      )
      setRanSql(trimmed)
      setRunning(false)
      if (wrote) onWrite?.()
      setHistory((cur) => {
        const updated = [trimmed, ...cur.filter((s) => s !== trimmed)].slice(0, HISTORY_LIMIT)
        saveHistory(db, updated)
        return updated
      })
    },
    [host, db, driver, running, onWrite, canLimit, limit],
  )

  const format = useCallback(async () => {
    if (!sql.trim() || formatting) return
    setFormatting(true)
    setFormatError(null)
    try {
      setSql(await formatSql(host, db, sql))
    } catch (err) {
      setFormatError(errText(err))
    } finally {
      setFormatting(false)
    }
  }, [host, db, sql, formatting])

  const selectionActions = useMemo<CodeEditorSelectionAction[]>(
    () => [
      {
        id: 'run-selection',
        label: 'Run selection',
        icon: <Play size={16} aria-hidden />,
        run: (selection) => void run(selection.text),
      },
    ],
    [run],
  )

  /**
   * The worker parses the plan. Prefixing `EXPLAIN` and rendering the result
   * as a grid of text was the old shape; `database::explain` normalises three
   * dialects into one tree and computes the warnings, so this only draws.
   */
  const explainNow = useCallback(async () => {
    const trimmed = sql.trim()
    if (!trimmed || running) return
    setRunning(true)
    setError(null)
    setSteps(null)
    try {
      setPlan(await explain(host, db, trimmed))
    } catch (err) {
      setPlan(null)
      setError(errText(err))
    } finally {
      setRunning(false)
    }
  }, [host, db, sql, running])

  // Judged per statement: a buffer of reads is not a write because it
  // holds several of them.
  const firstWrite = useMemo(
    () =>
      statements.length > 1
        ? statements.find((st) => !isReadOnlySql(st.text))?.text
        : sql.trim() !== '' && !isReadOnlySql(sql)
          ? sql
          : undefined,
    [statements, sql],
  )
  const isWrite = firstWrite !== undefined
  const ddl = useMemo(() => (firstWrite ? ddlInfo(firstWrite) : null), [firstWrite])
  const stale = outcome !== null && ranSql !== null && sql.trim() !== ranSql

  const clampSplit = (px: number) => {
    const total = panelRef.current?.getBoundingClientRect().height ?? 0
    const max = Math.max(MIN_EDITOR_PX, total - MIN_RESULTS_PX)
    return Math.min(max, Math.max(MIN_EDITOR_PX, px))
  }
  const commitSplit = (px: number | null) => {
    const next = px === null ? null : clampSplit(px)
    setEditorHeight(next)
    saveSplit(next)
  }
  const startSplitDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    const top = topRef.current
    if (!top) return
    e.preventDefault()
    const origin = top.getBoundingClientRect().top
    const handle = e.currentTarget
    handle.setPointerCapture(e.pointerId)
    const onMove = (move: PointerEvent) => setEditorHeight(clampSplit(move.clientY - origin))
    const onUp = (up: PointerEvent) => {
      commitSplit(up.clientY - origin)
      handle.removeEventListener('pointermove', onMove)
      handle.removeEventListener('pointerup', onUp)
      handle.removeEventListener('pointercancel', onUp)
    }
    handle.addEventListener('pointermove', onMove)
    handle.addEventListener('pointerup', onUp)
    handle.addEventListener('pointercancel', onUp)
  }

  const failedSql = step?.sql ?? ranSql
  const resultsLabel = error && (!steps || !outcome) ? 'Error' : plan ? 'Query plan' : 'Results'

  return (
    <div className="db-sql" ref={panelRef}>
      <div
        className={`db-sql-top${editorHeight !== null ? ' sized' : ''}`}
        ref={topRef}
        style={editorHeight !== null ? { flexBasis: editorHeight } : undefined}
      >
        {/* ⌘⏎ is caught on the way DOWN: Monaco binds it to "insert line
            below" and consumes it, so a bubbling handler never saw it and
            the chord typed a newline instead of running. */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: capture-phase chord for the editor inside; not a control */}
        <div
          className="db-sql-editor-wrap"
          onKeyDownCapture={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
              e.preventDefault()
              e.stopPropagation()
              void run(sql)
            }
          }}
        >
          <CodeEditor
            value={sql}
            onChange={setSql}
            language="sql"
            className="db-sql-code"
            fill
            lineNumbers
            fontSize={14}
            placeholder={`select * from … — ${RUN_CHORD} runs against ${db}`}
            aria-label="sql statement"
            completions={completions}
            selectionActions={selectionActions}
            ref={editorRef}
            onKeyDown={(e) => {
              // Monaco keeps Tab for indentation, so Escape is the keyboard
              // exit: land on the first usable action, or just release focus
              // when they're all disabled. (Monaco consumes Escape while a
              // widget is open — closing it — so this fires on the second.)
              if (e.key === 'Escape') {
                const next = actionsRef.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')
                if (next) next.focus()
                else if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
              }
            }}
          />
        </div>
        <div className="db-sql-actions" role="toolbar" aria-label="sql actions" ref={actionsRef}>
          <Button variant="ghost" size="sm" onClick={() => void explainNow()} disabled={running || sql.trim() === ''}>
            Explain
          </Button>
          {canFormat ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void format()}
              disabled={running || formatting || sql.trim() === ''}
            >
              <WandSparkles size={16} aria-hidden />
              {formatting ? 'Formatting…' : 'Format'}
            </Button>
          ) : null}
          <span ref={historyBtnRef} style={{ display: 'contents' }}>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setHistoryOpen((v) => !v)}
              disabled={history.length === 0}
              aria-expanded={historyOpen}
            >
              <History size={16} aria-hidden />
              History{history.length > 0 ? ` · ${history.length}` : ''}
            </Button>
          </span>
          {isWrite ? (
            // A heads-up, not a gate: this statement will mutate and commit.
            // Schema changes get the louder ink and name what they do.
            <span className={`db-sql-warn${ddl ? ' ddl' : ''}`}>
              {ddl ? `${ddl.present} — commits on ${db}` : `Write — runs and commits on ${db}`}
            </span>
          ) : null}
          {formatError ? (
            <span className="db-sql-warn" role="alert">
              Format failed — {formatError}
            </span>
          ) : null}
          <span className="db-toolbar-spacer" />
          {canLimit ? (
            <ShadcnSelect
              items={ROW_LIMITS.map((n) => ({
                value: String(n),
                label: n === 0 ? 'No limit' : `Limit ${n.toLocaleString()}`,
              }))}
              value={String(limit)}
              onValueChange={(next) => {
                if (next === null) return
                const n = Number(next)
                setLimit(n)
                saveLimit(n)
              }}
            >
              <ShadcnSelectTrigger size="sm" className="db-sql-limit" aria-label="row limit">
                <ShadcnSelectValue />
              </ShadcnSelectTrigger>
              <ShadcnSelectContent align="end">
                {ROW_LIMITS.map((n) => (
                  <ShadcnSelectItem key={n} value={String(n)}>
                    {n === 0 ? 'No limit' : `Limit ${n.toLocaleString()}`}
                  </ShadcnSelectItem>
                ))}
              </ShadcnSelectContent>
            </ShadcnSelect>
          ) : null}
          <Button
            variant="primary"
            size="sm"
            onClick={() => void run(sql)}
            disabled={running || sql.trim() === ''}
            title={statements.length > 1 ? 'Runs every statement in order; select text to run only that' : undefined}
          >
            {running ? (
              <LoaderCircle size={16} className={uiClasses.spin} aria-hidden />
            ) : (
              <Play size={16} aria-hidden />
            )}
            {running ? 'Running…' : statements.length > 1 ? `Run all · ${statements.length}` : 'Run'}
            <kbd className="db-sql-chord">{RUN_CHORD}</kbd>
          </Button>
          {/* The audible twin of the visual states: one polite live region
              voices running/result/failure and the write heads-up. The armed
              message is static on purpose — including the typed table name
              would re-announce on every keystroke. */}
          <span className="db-sr-only" role="status">
            {running
              ? 'running'
              : error
                ? `query failed: ${error}`
                : outcome
                  ? resultSummary(outcome)
                  : isWrite
                    ? `write statement — runs and commits on ${db}`
                    : ''}
          </span>
        </div>
        {historyOpen && history.length > 0 ? (
          <div className="db-sql-history" ref={historyRef}>
            {/* Where these live is worth one line: they never leave this
                browser, and eviction is silent at the cap. */}
            <div className="db-sql-history-note">Recent statements · Saved in this browser · Newest first</div>
            {history.map((entry) => (
              <div key={entry} className="db-sql-history-row">
                <button
                  type="button"
                  className="db-sql-history-pick"
                  onClick={() => {
                    setSql(entry)
                    setHistoryOpen(false)
                    editorRef.current?.focus()
                  }}
                  title={entry}
                >
                  {entry}
                </button>
                <IconButton
                  label="remove from history"
                  onClick={() =>
                    setHistory((cur) => {
                      const updated = cur.filter((s) => s !== entry)
                      saveHistory(db, updated)
                      return updated
                    })
                  }
                >
                  <X size={16} />
                </IconButton>
              </div>
            ))}
          </div>
        ) : null}
      </div>
      {/* biome-ignore lint/a11y/useSemanticElements: an <hr> cannot take pointer capture or arrow keys */}
      <div
        className="db-sql-split"
        role="separator"
        aria-orientation="horizontal"
        aria-label="resize the editor"
        aria-valuemin={MIN_EDITOR_PX}
        aria-valuenow={Math.round(editorHeight ?? topRef.current?.getBoundingClientRect().height ?? 0)}
        tabIndex={0}
        title="Drag to resize · double-click to reset"
        onPointerDown={startSplitDrag}
        onDoubleClick={() => commitSplit(null)}
        onKeyDown={(e) => {
          if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return
          e.preventDefault()
          const current = editorHeight ?? topRef.current?.getBoundingClientRect().height ?? MIN_EDITOR_PX
          commitSplit(current + (e.key === 'ArrowDown' ? SPLIT_STEP_PX : -SPLIT_STEP_PX))
        }}
      />
      <div className="db-sql-out">
        <div className="db-sql-reshead">
          <span className={`db-sql-reslabel${resultsLabel === 'Error' ? ' alert' : ''}`}>{resultsLabel}</span>
          {running ? (
            <span className="db-sql-resstate">
              <LoaderCircle size={16} className={uiClasses.spin} aria-hidden />
              Running{elapsed > 0 ? ` · ${elapsed}s` : '…'}
            </span>
          ) : outcome && !plan ? (
            <span className={`db-sql-resstate${stale ? ' stale' : ''}`}>
              {resultSummary(outcome)}
              {stale ? <span className="db-sql-stale-note"> · from an earlier version of the query</span> : null}
            </span>
          ) : null}
        </div>
        {steps && steps.length > 1 ? (
          <ol className="db-sql-steps" aria-label="statements run">
            {steps.map((s, i) => (
              <li key={`${i}:${s.sql}`}>
                <button
                  type="button"
                  className={`db-sql-step${s.error ? ' failed' : ''}${s.skipped ? ' skipped' : ''}`}
                  aria-current={i === stepIndex ? 'step' : undefined}
                  disabled={s.skipped}
                  onClick={() => setStepIndex(i)}
                  title={s.sql}
                >
                  <span className="idx">{i + 1}</span>
                  <span className="sql">{s.sql}</span>
                  <span className="meta">{s.result ? resultSummary(s.result) : s.skipped ? 'not run' : 'failed'}</span>
                </button>
              </li>
            ))}
          </ol>
        ) : null}
        <div className={`db-sql-results${running ? ' running' : ''}`} aria-busy={running || undefined}>
          {error && (!steps || !outcome) ? (
            <div className="db-pad">
              <StatusPanel variant="alert" icon={<CircleAlert size={18} />} headline="Query failed" detail={error} />
              {failedSql && !squash(error).includes(squash(failedSql)) ? (
                // What ran, beside why it failed — the editor may already
                // hold something else. Skipped when the driver's message
                // already quotes the statement.
                <pre className="db-sql-failed-stmt">{failedSql}</pre>
              ) : null}
            </div>
          ) : plan ? (
            <PlanTree plan={plan} />
          ) : outcome ? (
            outcome.write && outcome.result.rows.length === 0 ? (
              // A write without RETURNING has no grid to draw; an empty one
              // reads as "nothing happened", which is the opposite of the truth.
              <WriteOutcome
                outcome={outcome}
                sql={step?.sql ?? ranSql ?? ''}
                db={db}
                driver={driver}
                at={step?.at}
                tables={tables}
                onOpenTable={onOpenTable}
              />
            ) : (
              <ResultGrid
                columns={outcome.result.columns}
                rows={outcome.result.rows}
                rowCount={outcome.result.row_count}
                stickyHeader
                hideFooter
                rowNumbers
              />
            )
          ) : (
            <div className={`db-sql-empty${running ? ` ${uiClasses.pulse}` : ''}`}>
              {running ? (
                <p className="title">Running…</p>
              ) : (
                <>
                  <SquareTerminal size={16} className="db-sql-empty-icon" aria-hidden />
                  <p className="title">Run a query to see results</p>
                  <p className="hint">
                    <Kbd>{RUN_CHORD}</Kbd> runs the editor · select text to run only that part
                  </p>
                  {starterSql ? (
                    // The old copy suggested `select name from sqlite_master`
                    // whatever the driver was — an error on two of the three.
                    // A statement from the schema in front of you runs, and
                    // one click beats retyping it.
                    <Button
                      variant="secondary"
                      size="sm"
                      className="db-sql-starter"
                      onClick={() => {
                        setSql(starterSql)
                        void run(starterSql)
                      }}
                    >
                      <Play size={16} aria-hidden />
                      <code>{starterSql}</code>
                    </Button>
                  ) : null}
                </>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
})
