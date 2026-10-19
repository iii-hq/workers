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
  Select,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { CircleAlert, History, Play, WandSparkles, X } from 'lucide-react'
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react'
import { errText } from '../lib/errors'
import { type ExplainResult, explain, formatSql } from '../lib/rpc'
import { splitStatements } from '../lib/sql'
import {
  type AdhocResult,
  type DbDriver,
  ddlInfo,
  isReadOnlySql,
  resultSummary,
  runAdhocSql,
} from './db-data'
import { PlanTree } from './PlanTree'
import { ResultGrid } from './result-grid'

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
  error?: string
}


/** The run chord, written the way this keyboard has it. */
const RUN_CHORD =
  typeof navigator !== 'undefined' &&
  /mac/i.test(navigator.platform || navigator.userAgent)
    ? '⌘⏎'
    : 'ctrl+⏎'

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
    return Array.isArray(parsed)
      ? parsed.filter((s): s is string => typeof s === 'string')
      : []
  } catch {
    return []
  }
}

function saveHistory(db: string, entries: string[]) {
  try {
    window.localStorage.setItem(
      historyKey(db),
      JSON.stringify(entries.slice(0, HISTORY_LIMIT)),
    )
  } catch {
    // storage full/unavailable — history is a convenience, not state
  }
}

export const SqlPanel = forwardRef<SqlPanelHandle, SqlPanelProps>(
  function SqlPanel(
    {
      host,
      db,
      driver,
      canFormat,
      canLimit,
      seedSql,
      tables,
      starterSql,
      onWrite,
      onDirtyChange,
    },
    ref,
  ) {
    const [sql, setSql] = useState(seedSql ?? '')
    const completions = useMemo(
      () => Array.from(new Set([...(tables ?? []), ...SQL_KEYWORDS])),
      [tables],
    )
    const [running, setRunning] = useState(false)
    const [formatting, setFormatting] = useState(false)
    const [formatError, setFormatError] = useState<string | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [steps, setSteps] = useState<RunStep[] | null>(null)
    const [stepIndex, setStepIndex] = useState(0)
    const [limit, setLimit] = useState<number>(() => loadLimit())
    const statements = useMemo(
      () => splitStatements(sql, driver ?? 'unknown'),
      [sql, driver],
    )
    const step = steps?.[stepIndex] ?? null
    const outcome = step?.result ?? null
    const [history, setHistory] = useState<string[]>(() => loadHistory(db))
    const [historyOpen, setHistoryOpen] = useState(false)
    const [plan, setPlan] = useState<ExplainResult | null>(null)
    // The statement the current outcome belongs to — once the editor moves on,
    // the result line dims rather than passing off old numbers as current.
    const [ranSql, setRanSql] = useState<string | null>(null)
    const editorRef = useRef<CodeEditorHandle>(null)
    const actionsRef = useRef<HTMLDivElement>(null)
    const historyRef = useRef<HTMLDivElement>(null)
    const historyBtnRef = useRef<HTMLSpanElement>(null)
    useImperativeHandle(
      ref,
      () => ({ focus: () => editorRef.current?.focus() }),
      [],
    )

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
        if (
          historyRef.current?.contains(t) ||
          historyBtnRef.current?.contains(t)
        )
          return
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
          try {
            const result = await runAdhocSql(host, db, statement, { maxRows })
            if (result.write) wrote = true
            done.push({ sql: statement, result })
          } catch (err) {
            done.push({ sql: statement, error: errText(err) })
            break
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
          const updated = [trimmed, ...cur.filter((s) => s !== trimmed)].slice(
            0,
            HISTORY_LIMIT,
          )
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

    const isWrite = sql.trim() !== '' && !isReadOnlySql(sql)
    const ddl = useMemo(() => (isWrite ? ddlInfo(sql) : null), [isWrite, sql])
    const stale = outcome !== null && ranSql !== null && sql.trim() !== ranSql

    return (
      <div className="db-sql">
        <div className="db-sql-top">
          <div className="db-sql-editor-wrap">
            <CodeEditor
              value={sql}
              onChange={setSql}
              language="sql"
              className="db-sql-code"
              placeholder={`select * from … — ${RUN_CHORD} runs against ${db}`}
              aria-label="sql statement"
              completions={completions}
              selectionActions={selectionActions}
              ref={editorRef}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                  e.preventDefault()
                  void run(sql)
                }
                // Monaco keeps Tab for indentation, so Escape is the keyboard
                // exit: land on the first usable action, or just release focus
                // when they're all disabled. (Monaco consumes Escape while a
                // widget is open — closing it — so this fires on the second.)
                if (e.key === 'Escape') {
                  const next =
                    actionsRef.current?.querySelector<HTMLButtonElement>(
                      'button:not(:disabled)',
                    )
                  if (next) next.focus()
                  else if (document.activeElement instanceof HTMLElement)
                    document.activeElement.blur()
                }
              }}
            />
          </div>
          <div
            className={`${uiClasses.toolbar} db-bar`}
            role="toolbar"
            aria-label="sql actions"
            ref={actionsRef}
          >
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void run(sql)}
              disabled={running || sql.trim() === ''}
              title={
                statements.length > 1
                  ? 'Runs every statement in order; select text to run only that'
                  : undefined
              }
            >
              <Play size={16} aria-hidden />
              {running
                ? 'Running…'
                : statements.length > 1
                  ? `Run all · ${statements.length}`
                  : 'Run'}
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
            {canLimit ? (
              <Select
                appearance="inline"
                aria-label="row limit"
                sheetTitle="Row limit"
                value={String(limit)}
                options={ROW_LIMITS.map((n) => ({
                  value: String(n),
                  label: n === 0 ? 'No limit' : `Limit ${n.toLocaleString()}`,
                }))}
                onChange={(next) => {
                  const n = Number(next)
                  setLimit(n)
                  saveLimit(n)
                }}
              />
            ) : null}
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void explainNow()}
              disabled={running || sql.trim() === ''}
            >
              Explain
            </Button>
            <span ref={historyBtnRef} style={{ display: 'contents' }}>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setHistoryOpen((v) => !v)}
                disabled={history.length === 0}
                aria-expanded={historyOpen}
              >
                <History size={16} aria-hidden />
                History · {history.length}
              </Button>
            </span>
            {isWrite ? (
              // A heads-up, not a gate: this statement will mutate and commit.
              // Schema changes get the louder ink and name what they do.
              <span className={`db-sql-warn${ddl ? ' ddl' : ''}`}>
                {ddl
                  ? `${ddl.present} — commits on ${db}`
                  : `write — runs and commits on ${db}`}
              </span>
            ) : null}
            {formatError ? (
              <span className="db-sql-warn" role="alert">
                format failed — {formatError}
              </span>
            ) : null}
            {outcome ? (
              <span className={`db-sql-meta${stale ? ' stale' : ''}`}>
                {resultSummary(outcome)}
              </span>
            ) : null}
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
              <div className="db-sql-history-note">
                Recent statements · Saved in this browser · Newest first
              </div>
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
        <div className={`db-sql-results${running ? ' running' : ''}`}>
          {steps && steps.length > 1 ? (
            <ol className="db-sql-steps" aria-label="statements run">
              {steps.map((s, i) => (
                <li key={`${i}:${s.sql}`}>
                  <button
                    type="button"
                    className={`db-sql-step${s.error ? ' failed' : ''}`}
                    aria-current={i === stepIndex ? 'step' : undefined}
                    onClick={() => setStepIndex(i)}
                    title={s.sql}
                  >
                    <span className="idx">{i + 1}</span>
                    <span className="sql">{s.sql}</span>
                    <span className="meta">
                      {s.result ? resultSummary(s.result) : 'failed'}
                    </span>
                  </button>
                </li>
              ))}
            </ol>
          ) : null}
          {error && (!steps || !outcome) ? (
            <div className="db-pad">
              <StatusPanel
                variant="alert"
                icon={<CircleAlert size={18} />}
                headline="Query failed"
                detail={error}
              />
            </div>
          ) : plan ? (
            <PlanTree plan={plan} />
          ) : outcome ? (
            outcome.write && outcome.result.rows.length === 0 ? (
              // A write without RETURNING has no grid to draw; an empty one
              // reads as "nothing happened", which is the opposite of the truth.
              <p className="db-sql-placeholder">
                Statement ran —{' '}
                {outcome.write.echo ??
                  `${outcome.write.affectedRows} row${outcome.write.affectedRows === 1 ? '' : 's'} affected`}
              </p>
            ) : (
              <ResultGrid
                columns={outcome.result.columns}
                rows={outcome.result.rows}
                rowCount={outcome.result.row_count}
                stickyHeader
              />
            )
          ) : (
            <p
              className={`db-sql-placeholder${running ? ` ${uiClasses.pulse}` : ''}`}
            >
              {running ? (
                'running…'
              ) : starterSql ? (
                <>
                  results appear here. try{' '}
                  {/* The old copy suggested `select name from sqlite_master`
                    whatever the driver was — an error on two of the three.
                    A statement from the schema in front of you runs, and one
                    click beats retyping it. */}
                  <button
                    type="button"
                    className="db-linkish"
                    onClick={() => {
                      setSql(starterSql)
                      void run(starterSql)
                    }}
                  >
                    {starterSql}
                  </button>
                </>
              ) : (
                'results appear here.'
              )}
            </p>
          )}
        </div>
      </div>
    )
  },
)
