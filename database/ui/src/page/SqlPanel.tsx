/**
 * Ad-hoc read-only SQL: the shared Monaco `CodeEditor` (⌘⏎ runs), an EXPLAIN
 * action that prefixes the driver-native explain form, per-database history
 * in localStorage, and results in the shared grid with duration. Write verbs
 * never leave the browser — `runReadOnlySql` rejects them.
 */

import {
  Button,
  CodeEditor,
  type CodeEditorHandle,
  type Host,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  type AdhocResult,
  type DbDriver,
  explainPrefix,
  isReadOnlySql,
  runReadOnlySql,
} from './db-data'
import { AlertCircle, History, Play, X } from './icons'
import { ResultGrid } from './result-grid'

interface SqlPanelProps {
  host: Host
  db: string
  driver: DbDriver
  /** Prefill from "query this table" affordances; applied when it changes. */
  seedSql?: string
  /** Table names in the active database — fed to the editor's autocomplete
      alongside the SQL keywords. */
  tables?: readonly string[]
}

const HISTORY_LIMIT = 20

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

export function SqlPanel({ host, db, driver, seedSql, tables }: SqlPanelProps) {
  const [sql, setSql] = useState(seedSql ?? '')
  const completions = useMemo(
    () => Array.from(new Set([...(tables ?? []), ...SQL_KEYWORDS])),
    [tables],
  )
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [outcome, setOutcome] = useState<AdhocResult | null>(null)
  const [history, setHistory] = useState<string[]>(() => loadHistory(db))
  const [historyOpen, setHistoryOpen] = useState(false)
  const editorRef = useRef<CodeEditorHandle>(null)

  useEffect(() => {
    if (seedSql) setSql(seedSql)
  }, [seedSql])

  const run = useCallback(
    async (statement: string) => {
      const trimmed = statement.trim()
      if (!trimmed || running) return
      setRunning(true)
      setError(null)
      try {
        const next = await runReadOnlySql(host, db, trimmed)
        setOutcome(next)
        setHistory((cur) => {
          const updated = [trimmed, ...cur.filter((s) => s !== trimmed)].slice(
            0,
            HISTORY_LIMIT,
          )
          saveHistory(db, updated)
          return updated
        })
      } catch (err) {
        setOutcome(null)
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setRunning(false)
      }
    },
    [host, db, running],
  )

  const readOnly = sql.trim() === '' || isReadOnlySql(sql)

  return (
    <div className="db-sql">
      <div className="db-sql-top">
        <div className="db-sql-editor-wrap">
          <CodeEditor
            value={sql}
            onChange={setSql}
            language="sql"
            className="db-sql-code"
            placeholder={`select * from … — read-only, ⌘⏎ runs against ${db}`}
            aria-label="sql statement"
            completions={completions}
            ref={editorRef}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                e.preventDefault()
                void run(sql)
              }
            }}
          />
        </div>
        <div className="db-sql-actions">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void run(sql)}
            disabled={running || sql.trim() === ''}
          >
            <Play size={12} aria-hidden />
            {running ? 'running…' : 'run'}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void run(`${explainPrefix(driver)}${sql.trim()}`)}
            disabled={running || sql.trim() === ''}
          >
            explain
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setHistoryOpen((v) => !v)}
            disabled={history.length === 0}
          >
            <History size={12} aria-hidden />
            history · {history.length}
          </Button>
          {!readOnly ? (
            <span className="db-sql-warn">
              write statements are rejected — this panel is read-only
            </span>
          ) : null}
          {outcome ? (
            <span className="db-sql-meta">
              {outcome.result.row_count} row
              {outcome.result.row_count === 1 ? '' : 's'} · {outcome.durationMs}
              ms
            </span>
          ) : null}
        </div>
        {historyOpen && history.length > 0 ? (
          <div className="db-sql-history">
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
      <div className="db-sql-results">
        {error ? (
          <div className="db-pad">
            <StatusPanel
              variant="alert"
              icon={<AlertCircle size={18} />}
              headline="query failed"
              detail={error}
            />
          </div>
        ) : outcome ? (
          <ResultGrid
            columns={outcome.result.columns}
            rows={outcome.result.rows}
            rowCount={outcome.result.row_count}
            stickyHeader
          />
        ) : (
          <p className={`db-sql-placeholder${running ? ' db-pulse' : ''}`}>
            {running
              ? 'running…'
              : 'results appear here. try: select name from sqlite_master limit 10'}
          </p>
        )}
      </div>
    </div>
  )
}
