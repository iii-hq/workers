/**
 * Custom configuration form for the `database` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * schema-driven form for this worker only.
 *
 * One card per configured database: connection URL with a live driver
 * badge, capture mode with driver-aware guidance, TLS (hidden for sqlite,
 * which ignores it), and the pool knobs. The form edits the working draft
 * via `onChange`; dirty tracking, save/reset, validation and the SaveBar
 * stay host-owned. Mirrors DatabaseConfig (database/src/config.rs).
 */

import { useEffect, useRef, useState } from 'react'
import type { ConfigFormProps, Host, JsonValue } from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

type Driver = 'postgres' | 'mysql' | 'sqlite' | 'unknown'

function driverOf(url: string): Driver {
  if (url.startsWith('postgres://') || url.startsWith('postgresql://')) return 'postgres'
  if (url.startsWith('mysql://')) return 'mysql'
  if (url.startsWith('sqlite:')) return 'sqlite'
  return 'unknown'
}

const CAPTURE_HINTS: Record<Driver, string> = {
  postgres:
    'native: any client’s committed writes fire database::row-changed via triggers + LISTEN/NOTIFY. The role needs DDL rights on watched tables; bindings must name a table.',
  sqlite:
    'native: triggers + changelog table + filesystem watch hear every process writing the file. Bindings must name a table.',
  mysql:
    'native: streams the binlog as a replica — nothing installed in the schema, but the user needs GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.*',
  unknown: 'set a url first — capture support depends on the driver.',
}

const POOL_FIELDS = [
  { key: 'max', label: 'max connections', placeholder: '10' },
  { key: 'idle_timeout_ms', label: 'idle timeout (ms)', placeholder: '30000' },
  { key: 'acquire_timeout_ms', label: 'acquire timeout (ms)', placeholder: '5000' },
] as const

/** Wire shape of `database::testConnection`. */
interface TestConnectionResp {
  ok: boolean
  driver: string
  latency_ms: number
  server_version?: string
  message?: string
}

type TestResult = { status: 'testing' } | { status: 'done'; ok: boolean; text: string }

export function DatabaseConfigForm(props: ConfigFormProps & { host: Host }) {
  const value = asObject(props.value)
  const databases = asObject(value.databases)
  const names = Object.keys(databases)

  // Renames commit on blur: committing per keystroke would collide with a
  // sibling entry mid-typing and silently swallow it.
  const [pendingNames, setPendingNames] = useState<Record<string, string>>({})
  // Probe outcomes are keyed by handle and dropped on any edit of that
  // handle — a stale "connected" next to a changed url would be a lie.
  const [testResults, setTestResults] = useState<Record<string, TestResult>>({})

  const commit = (nextDatabases: JsonObject) =>
    props.onChange({ ...value, databases: nextDatabases })

  const clearTest = (name: string) =>
    setTestResults((r) => {
      const next = { ...r }
      delete next[name]
      return next
    })

  const setDb = (name: string, next: JsonObject) => {
    clearTest(name)
    commit({ ...databases, [name]: next })
  }

  const runTest = async (name: string) => {
    const db = asObject(databases[name])
    setTestResults((r) => ({ ...r, [name]: { status: 'testing' } }))
    let result: TestResult
    try {
      const resp = await props.host.iii.trigger<TestConnectionResp>(
        'database::testConnection',
        { url: asString(db.url), tls: db.tls ?? undefined, timeout_ms: 8000 },
        { timeoutMs: 10_000 },
      )
      result = {
        status: 'done',
        ok: resp.ok,
        text: resp.ok
          ? `connected · ${resp.server_version ?? resp.driver} · ${resp.latency_ms}ms`
          : resp.message ?? 'connection failed',
      }
    } catch (e) {
      result = { status: 'done', ok: false, text: e instanceof Error ? e.message : String(e) }
    }
    setTestResults((r) => ({ ...r, [name]: result }))
  }

  const removeDb = (name: string) => {
    clearTest(name)
    const next = { ...databases }
    delete next[name]
    commit(next)
  }

  const addDb = () => {
    let i = names.length + 1
    let name = names.length === 0 ? 'primary' : `db${i}`
    while (databases[name] !== undefined) name = `db${++i}`
    commit({ ...databases, [name]: { url: `sqlite:./data/${name}.db` } })
  }

  const renameDb = (from: string, to: string) => {
    const trimmed = to.trim()
    setPendingNames((p) => {
      const next = { ...p }
      delete next[from]
      return next
    })
    if (trimmed === '' || trimmed === from || databases[trimmed] !== undefined) return
    clearTest(from)
    // Rebuild in place so the card doesn't jump to the end of the list.
    const next: JsonObject = {}
    for (const key of names) {
      next[key === from ? trimmed : key] = databases[key]
    }
    commit(next)
  }

  // Deep-link focus (`#/workers/configuration/database/<field>`): a custom
  // form honors `focusField` itself. First segment `databases` + a name
  // scrolls that card; anything else matches a `data-field` directly.
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const path = props.focusField
    if (!path || path.length === 0 || !rootRef.current) return
    const selector =
      path[0] === 'databases' && path[1]
        ? `[data-field="db-${path[1]}"]`
        : `[data-field="${path[0]}"]`
    const target = rootRef.current.querySelector<HTMLElement>(selector)
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="db-cfg" ref={rootRef}>
      <span className="db-cfg-caption">custom form · shipped by the database worker</span>

      {names.length === 0 ? (
        <div className="db-cfg-empty">No databases configured — the worker refuses to start without at least one.</div>
      ) : null}

      {names.map((name) => (
        <DatabaseCard
          key={name}
          name={name}
          db={asObject(databases[name])}
          pendingName={pendingNames[name]}
          onPendingName={(v) => setPendingNames((p) => ({ ...p, [name]: v }))}
          onRename={(to) => renameDb(name, to)}
          onChange={(next) => setDb(name, next)}
          onRemove={() => removeDb(name)}
          removable={names.length > 1}
          test={testResults[name]}
          onTest={() => runTest(name)}
        />
      ))}

      <button type="button" className="db-cfg-add" onClick={addDb}>
        + add database
      </button>

      {props.errors && props.errors.size > 0 ? (
        <div className="db-cfg-errors">
          {[...props.errors.entries()].map(([pointer, message]) => (
            <div key={pointer}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function DatabaseCard(card: {
  name: string
  db: JsonObject
  pendingName: string | undefined
  onPendingName: (v: string) => void
  onRename: (to: string) => void
  onChange: (next: JsonObject) => void
  onRemove: () => void
  removable: boolean
  test: TestResult | undefined
  onTest: () => void
}) {
  const { name, db } = card
  const url = asString(db.url)
  const driver = driverOf(url)
  const capture = asString(db.capture) || 'statements'
  const tls = asObject(db.tls)
  const pool = asObject(db.pool)
  const isMemorySqlite = driver === 'sqlite' && url.includes(':memory:')

  const set = (mutate: (next: JsonObject) => void) => {
    const next = { ...db }
    mutate(next)
    card.onChange(next)
  }

  const setBlock = (key: 'tls' | 'pool', mutate: (block: JsonObject) => void) =>
    set((next) => {
      const block = asObject(next[key])
      mutate(block)
      if (Object.keys(block).length > 0) next[key] = block
      else delete next[key]
    })

  return (
    <section className="db-cfg-card" data-field={`db-${name}`} tabIndex={-1}>
      <header className="db-cfg-card-head">
        <input
          className="db-cfg-name"
          type="text"
          value={card.pendingName ?? name}
          aria-label="database handle"
          onChange={(e) => card.onPendingName(e.target.value)}
          onBlur={(e) => card.onRename(e.target.value)}
        />
        <span className={`db-cfg-driver db-cfg-driver-${driver}`}>{driver}</span>
        {capture === 'native' ? <span className="db-cfg-capture-pill">native capture</span> : null}
        <span className="db-cfg-spacer" />
        {card.removable ? (
          <button type="button" className="db-cfg-remove" onClick={card.onRemove}>
            remove
          </button>
        ) : null}
      </header>

      <div className="db-cfg-field">
        <label htmlFor={`db-cfg-url-${name}`}>connection url</label>
        <div className="db-cfg-url-row">
          <input
            id={`db-cfg-url-${name}`}
            data-field={`databases-${name}-url`}
            className="db-cfg-input db-cfg-grow"
            type="text"
            value={url}
            placeholder="postgres://user:pass@host:5432/db · mysql://… · sqlite:./data/app.db"
            onChange={(e) => set((next) => (next.url = e.target.value))}
          />
          <button
            type="button"
            className="db-cfg-test"
            disabled={card.test?.status === 'testing' || url.trim() === ''}
            onClick={card.onTest}
          >
            {card.test?.status === 'testing' ? 'testing…' : 'test connection'}
          </button>
        </div>
        {card.test?.status === 'done' ? (
          <span className={card.test.ok ? 'db-cfg-test-ok' : 'db-cfg-test-fail'}>
            {card.test.text}
          </span>
        ) : null}
      </div>

      <div className="db-cfg-field">
        <label htmlFor={`db-cfg-capture-${name}`}>row-change capture</label>
        <select
          id={`db-cfg-capture-${name}`}
          className="db-cfg-select"
          value={capture}
          onChange={(e) =>
            set((next) => {
              if (e.target.value === 'native') next.capture = 'native'
              else delete next.capture
            })
          }
        >
          <option value="statements">statements — only writes made through this worker</option>
          <option value="native">native — writes from any client, including other processes</option>
        </select>
        {capture === 'native' && isMemorySqlite ? (
          <span className="db-cfg-warn">
            a `:memory:` database is per-connection and cannot be captured — the worker rejects
            this configuration
          </span>
        ) : (
          <span className="db-cfg-hint">{CAPTURE_HINTS[driver]}</span>
        )}
      </div>

      {driver === 'postgres' || driver === 'mysql' ? (
        <div className="db-cfg-row">
          <div className="db-cfg-field">
            <label htmlFor={`db-cfg-tls-${name}`}>tls</label>
            <select
              id={`db-cfg-tls-${name}`}
              className="db-cfg-select"
              value={asString(tls.mode) || 'require'}
              onChange={(e) =>
                setBlock('tls', (block) => {
                  if (e.target.value === 'require') delete block.mode
                  else block.mode = e.target.value
                })
              }
            >
              <option value="disable">disable — plaintext (local dev only)</option>
              <option value="require">require — TLS, chain validated (default)</option>
              <option value="verify-full">verify-full — also verify hostname</option>
            </select>
          </div>
          <div className="db-cfg-field db-cfg-grow">
            <label htmlFor={`db-cfg-ca-${name}`}>extra CA bundle (PEM path, optional)</label>
            <input
              id={`db-cfg-ca-${name}`}
              className="db-cfg-input"
              type="text"
              value={asString(tls.ca_cert)}
              placeholder="/etc/ssl/private-ca.pem"
              onChange={(e) =>
                setBlock('tls', (block) => {
                  if (e.target.value === '') delete block.ca_cert
                  else block.ca_cert = e.target.value
                })
              }
            />
          </div>
        </div>
      ) : null}

      <details className="db-cfg-pool">
        <summary>connection pool</summary>
        <div className="db-cfg-row">
          {POOL_FIELDS.map((f) => (
            <div className="db-cfg-field" key={f.key}>
              <label htmlFor={`db-cfg-${f.key}-${name}`}>{f.label}</label>
              <input
                id={`db-cfg-${f.key}-${name}`}
                className="db-cfg-input"
                type="number"
                min={1}
                value={typeof pool[f.key] === 'number' ? (pool[f.key] as number) : ''}
                placeholder={f.placeholder}
                onChange={(e) =>
                  setBlock('pool', (block) => {
                    if (e.target.value.trim() === '') delete block[f.key]
                    else if (!Number.isNaN(Number(e.target.value))) block[f.key] = Number(e.target.value)
                  })
                }
              />
            </div>
          ))}
        </div>
      </details>
    </section>
  )
}
