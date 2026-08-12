/**
 * Ad-hoc SQL: the shared Monaco `CodeEditor` (⌘⏎ runs), an explain action,
 * per-database history in localStorage, and results in the shared grid with
 * duration. Reads run through `database::query`, writes through
 * `database::execute` — a "write" note appears by the actions before a
 * mutating statement runs, and the result line reports affected rows.
 *
 * Explain goes through `database::explain`, which parses the dialect's own
 * plan format into one tree. The panel no longer knows which driver it is
 * talking to, which is why there is no `driver` prop.
 */

import { Button, CodeEditor, type CodeEditorHandle, type Host, StatusPanel } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errText } from '../lib/errors'
import { type ExplainResult, explain } from '../lib/rpc'
import { type AdhocResult, ddlInfo, isReadOnlySql, runAdhocSql } from './db-data'
import { AlertCircle, History, Play, X } from './icons'
import { PlanTree } from './PlanTree'
import { ResultGrid } from './result-grid'

interface SqlPanelProps {
  host: Host
  db: string
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

const HISTORY_LIMIT = 20

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

export function SqlPanel({ host, db, seedSql, tables, starterSql, onWrite, onDirtyChange }: SqlPanelProps) {
  const [sql, setSql] = useState(seedSql ?? '')
  const completions = useMemo(() => Array.from(new Set([...(tables ?? []), ...SQL_KEYWORDS])), [tables])
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [outcome, setOutcome] = useState<AdhocResult | null>(null)
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

  const run = useCallback(
    async (statement: string) => {
      const trimmed = statement.trim()
      if (!trimmed || running) return
      setRunning(true)
      setError(null)
      setPlan(null)
      try {
        const next = await runAdhocSql(host, db, trimmed)
        setOutcome(next)
        setRanSql(trimmed)
        if (next.write) onWrite?.()
        setHistory((cur) => {
          const updated = [trimmed, ...cur.filter((s) => s !== trimmed)].slice(0, HISTORY_LIMIT)
          saveHistory(db, updated)
          return updated
        })
      } catch (err) {
        setOutcome(null)
        setError(errText(err))
      } finally {
        setRunning(false)
      }
    },
    [host, db, running, onWrite],
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
    setOutcome(null)
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
                const next = actionsRef.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')
                if (next) next.focus()
                else if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
              }
            }}
          />
        </div>
        <div className="db-sql-actions" ref={actionsRef}>
          <Button variant="ghost" size="sm" onClick={() => void run(sql)} disabled={running || sql.trim() === ''}>
            <Play size={12} aria-hidden />
            {running ? 'running…' : 'run'}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void explainNow()} disabled={running || sql.trim() === ''}>
            explain
          </Button>
          <span ref={historyBtnRef} style={{ display: 'contents' }}>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setHistoryOpen((v) => !v)}
              disabled={history.length === 0}
              aria-expanded={historyOpen}
            >
              <History size={12} aria-hidden />
              history · {history.length}
            </Button>
          </span>
          {isWrite ? (
            // A heads-up, not a gate: this statement will mutate and commit.
            // Schema changes get the louder ink and name what they do.
            <span className={`db-sql-warn${ddl ? ' ddl' : ''}`}>
              {ddl ? `${ddl.present} — commits on ${db}` : `write — runs and commits on ${db}`}
            </span>
          ) : null}
          {outcome ? (
            <span className={`db-sql-meta${stale ? ' stale' : ''}`}>
              {outcome.write
                ? (outcome.write.echo ??
                  `${outcome.write.affectedRows} affected${
                    outcome.write.lastInsertId !== null ? ` · id ${outcome.write.lastInsertId}` : ''
                  }`)
                : `${outcome.result.row_count} row${outcome.result.row_count === 1 ? '' : 's'}`}{' '}
              · {outcome.durationMs}
              ms
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
                  ? outcome.write
                    ? `${outcome.write.echo ?? `${outcome.write.affectedRows} rows affected`} in ${outcome.durationMs}ms`
                    : `${outcome.result.row_count} rows in ${outcome.durationMs}ms`
                  : isWrite
                    ? `write statement — runs and commits on ${db}`
                    : ''}
          </span>
        </div>
        {historyOpen && history.length > 0 ? (
          <div className="db-sql-history" ref={historyRef}>
            {/* Where these live is worth one line: they never leave this
                browser, and eviction is silent at the cap. */}
            <div className="db-sql-history-note">recent statements · saved in this browser · newest first</div>
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
                <button
                  type="button"
                  className="db-icon-btn"
                  onClick={() =>
                    setHistory((cur) => {
                      const updated = cur.filter((s) => s !== entry)
                      saveHistory(db, updated)
                      return updated
                    })
                  }
                  aria-label="remove from history"
                >
                  <X size={12} />
                </button>
              </div>
            ))}
          </div>
        ) : null}
      </div>
      <div className={`db-sql-results${running ? ' running' : ''}`}>
        {error ? (
          <div className="db-pad">
            <StatusPanel variant="alert" icon={<AlertCircle size={18} />} headline="query failed" detail={error} />
          </div>
        ) : plan ? (
          <PlanTree plan={plan} />
        ) : outcome ? (
          outcome.write && outcome.result.rows.length === 0 ? (
            // A write without RETURNING has no grid to draw; an empty one
            // reads as "nothing happened", which is the opposite of the truth.
            <p className="db-sql-placeholder">
              statement ran —{' '}
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
          <p className={`db-sql-placeholder${running ? ' db-pulse' : ''}`}>
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
}
