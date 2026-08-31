/**
 * Custom configuration form for the `database` configuration entry —
 * registered through `host.configForms` as the worker-owned editor.
 *
 * One card per configured database: connection URL with a live driver
 * badge, capture mode with driver-aware guidance, TLS (hidden for sqlite,
 * which ignores it), and the pool knobs. The form edits the working draft
 * via `onChange`; dirty tracking, save/reset, validation and the SaveBar
 * stay host-owned. Mirrors DatabaseConfig (database/src/config.rs).
 */

import {
  type ConfigFormProps,
  type Host,
  type JsonValue,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { errText } from '../lib/errors'
import {
  booleanLiteralForRawValue,
  DEFAULT_HISTORY_MAX_BYTES,
  DEFAULT_HISTORY_MAX_ENTRIES,
  type Driver,
  databaseFocusRequest,
  driverOfConfiguredUrl,
  isEnvironmentValue,
  isRawTypedValue,
  numberLiteralForRawValue,
  selectLiteralForRawValue,
  shouldMaskConfiguredUrl,
  shouldShowTlsForUrl,
} from './model'

type JsonObject = { [key: string]: JsonValue }
const CONFIG_NARROW_BELOW = 760

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

function asNumber(v: JsonValue | undefined, fallback: number): number {
  if (typeof v === 'number') return v
  return typeof v === 'string' ? numberLiteralForRawValue(v, fallback) : fallback
}

function asBoolean(v: JsonValue | undefined, fallback: boolean): boolean {
  if (typeof v === 'boolean') return v
  return typeof v === 'string' ? booleanLiteralForRawValue(v, fallback) : fallback
}

function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const ref = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return

      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      if (typeof ResizeObserver === 'undefined') return

      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )

  return [ref, narrow]
}

export function focusDatabaseDetail(root: HTMLElement | null): boolean {
  const target = root?.querySelector<HTMLElement>('.db-cfg-detail')
  if (!target) return false
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'start' })
  return true
}

function pointer(parts: readonly string[]) {
  return `/${parts.map((part) => part.replaceAll('~', '~0').replaceAll('/', '~1')).join('/')}`
}

function fieldError(errors: ConfigFormProps['errors'], parts: readonly string[]) {
  const base = pointer(parts)
  return errors?.get(base) ?? [...(errors?.entries() ?? [])].find(([path]) => path.startsWith(`${base}/`))?.[1]
}

const CAPTURE_HINTS: Record<Driver, string> = {
  postgres:
    'Native: any client’s committed writes fire database::row-changed via triggers + LISTEN/NOTIFY. The role needs DDL rights on watched tables; bindings must name a table.',
  sqlite:
    'Native: triggers + changelog table + filesystem watch hear every process writing the file. Bindings must name a table.',
  mysql:
    'Native: streams the binlog as a replica — nothing installed in the schema, but the user needs GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.*',
  unknown: 'Set a URL first — capture support depends on the driver.',
}

const POOL_FIELDS = [
  {
    key: 'max',
    label: 'Maximum connections',
    description: 'Upper bound for open connections in this pool.',
    placeholder: '10',
  },
  {
    key: 'idle_timeout_ms',
    label: 'Idle timeout',
    description: 'Close unused connections after this many milliseconds.',
    placeholder: '30000',
  },
  {
    key: 'acquire_timeout_ms',
    label: 'Acquire timeout',
    description: 'Wait this many milliseconds for a connection before failing.',
    placeholder: '5000',
  },
] as const

const DRIVER_LABELS: Readonly<Record<Driver, string>> = {
  postgres: 'PostgreSQL',
  mysql: 'MySQL',
  sqlite: 'SQLite',
  unknown: 'Unknown',
}

/** Wire shape of `database::testConnection`. */
interface TestConnectionResp {
  ok: boolean
  driver: string
  latency_ms: number
  server_version?: string
  message?: string
}

type TestResult = { status: 'testing' } | { status: 'done'; ok: boolean; text: string }

function RawTypedValue({
  id,
  name,
  dataField,
  label,
  value,
  replacementLabel,
  error,
  errorId,
  onChange,
  onUseLiteral,
}: {
  id: string
  name?: string
  dataField: string
  label: string
  value: string
  replacementLabel: string
  error?: string
  errorId?: string
  onChange(next: string): void
  onUseLiteral(): void
}) {
  const environmentBacked = isEnvironmentValue(value) || /^\$\{[^}]*$/.test(value)
  return (
    <div className="db-cfg-template-control" data-environment-template={environmentBacked ? 'true' : 'false'}>
      <span className="db-cfg-template-kind">{environmentBacked ? 'Environment' : 'Custom value'}</span>
      <input
        id={id}
        name={name}
        data-field={dataField}
        className="db-cfg-input db-cfg-template-input"
        type="text"
        value={value}
        spellCheck={false}
        autoComplete="off"
        aria-label={`${label} raw value`}
        aria-invalid={error ? true : undefined}
        aria-describedby={errorId}
        onChange={(event) => onChange(event.target.value)}
      />
      <button
        type="button"
        className="db-cfg-template-replace"
        onClick={onUseLiteral}
        aria-label={`Replace ${label} environment value with ${replacementLabel}`}
      >
        Use {replacementLabel}
      </button>
    </div>
  )
}

function DatabaseNumberInput({
  id,
  name,
  dataField,
  label,
  value,
  fallback,
  error,
  errorId,
  className = 'db-cfg-input',
  onChange,
}: {
  id: string
  name?: string
  dataField: string
  label: string
  value: JsonValue | undefined
  fallback: number
  error?: string
  errorId?: string
  className?: string
  onChange(raw: string): void
}) {
  if (isRawTypedValue(value)) {
    const replacement = numberLiteralForRawValue(value, fallback)
    return (
      <RawTypedValue
        id={id}
        name={name}
        dataField={dataField}
        label={label}
        value={value}
        replacementLabel={String(replacement)}
        error={error}
        errorId={errorId}
        onChange={onChange}
        onUseLiteral={() => onChange(String(replacement))}
      />
    )
  }
  return (
    <input
      id={id}
      name={name}
      data-field={dataField}
      className={className}
      type="number"
      min={0}
      inputMode="numeric"
      value={typeof value === 'number' ? value : ''}
      placeholder={String(fallback)}
      aria-label={label}
      aria-invalid={error ? true : undefined}
      aria-describedby={errorId}
      onChange={(event) => onChange(event.target.value)}
    />
  )
}

function DatabaseSelectInput({
  id,
  name,
  dataField,
  label,
  value,
  fallback,
  options,
  error,
  errorId,
  onChange,
}: {
  id: string
  name: string
  dataField: string
  label: string
  value: JsonValue | undefined
  fallback: string
  options: readonly { value: string; label: string }[]
  error?: string
  errorId?: string
  onChange(next: string): void
}) {
  const current = typeof value === 'string' ? value : fallback
  const knownValue = options.some((option) => option.value === current)
  if (!knownValue) {
    const replacement = selectLiteralForRawValue(
      current,
      options.map((option) => option.value),
      fallback,
    )
    const replacementLabel = options.find((option) => option.value === replacement)?.label ?? replacement
    return (
      <RawTypedValue
        id={id}
        name={name}
        dataField={dataField}
        label={label}
        value={current}
        replacementLabel={replacementLabel}
        error={error}
        errorId={errorId}
        onChange={onChange}
        onUseLiteral={() => onChange(replacement)}
      />
    )
  }
  return (
    <select
      id={id}
      name={name}
      data-field={dataField}
      className="db-cfg-select"
      value={current}
      aria-invalid={error ? true : undefined}
      aria-describedby={errorId}
      onChange={(event) => onChange(event.target.value)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  )
}

function HistorySettings({
  value,
  errors,
  onChange,
}: {
  value: JsonObject
  errors: ConfigFormProps['errors']
  onChange: (value: JsonObject) => void
}) {
  const entriesError = fieldError(errors, ['history_max_entries'])
  const bytesError = fieldError(errors, ['history_max_bytes'])
  const entriesErrorId = entriesError ? 'db-cfg-history-max-entries-error' : undefined
  const bytesErrorId = bytesError ? 'db-cfg-history-max-bytes-error' : undefined
  const update = (field: 'history_max_entries' | 'history_max_bytes', raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else {
      const parsed = Number(raw)
      next[field] = Number.isInteger(parsed) && parsed >= 0 ? parsed : raw
    }
    onChange(next)
  }

  return (
    <SettingsSection
      className="db-cfg-history"
      title="Query history"
      description="Per-database history is stored by the state worker. The first limit reached removes the oldest entries."
    >
      <SettingsList>
        <SettingsRow
          layout="auto"
          label="Maximum entries"
          description="Set 0 to disable query history recording."
          meta={`${asNumber(value.history_max_entries, DEFAULT_HISTORY_MAX_ENTRIES).toLocaleString('en-US')} entries per database`}
          control={
            <DatabaseNumberInput
              id="db-cfg-history-max-entries"
              name="history_max_entries"
              dataField="history_max_entries"
              label="Maximum query history entries"
              value={value.history_max_entries}
              fallback={DEFAULT_HISTORY_MAX_ENTRIES}
              error={entriesError}
              errorId={entriesErrorId}
              className="db-cfg-input db-cfg-history-input"
              onChange={(raw) => update('history_max_entries', raw)}
            />
          }
        />
        <SettingsRow
          layout="auto"
          label="Maximum storage"
          description="Maximum JSON-serialized history size. Set 0 to disable recording."
          meta={`${asNumber(value.history_max_bytes, DEFAULT_HISTORY_MAX_BYTES).toLocaleString('en-US')} bytes per database`}
          control={
            <DatabaseNumberInput
              id="db-cfg-history-max-bytes"
              name="history_max_bytes"
              dataField="history_max_bytes"
              label="Maximum query history storage in bytes"
              value={value.history_max_bytes}
              fallback={DEFAULT_HISTORY_MAX_BYTES}
              error={bytesError}
              errorId={bytesErrorId}
              className="db-cfg-input db-cfg-history-input"
              onChange={(raw) => update('history_max_bytes', raw)}
            />
          }
        />
      </SettingsList>
      {entriesError ? (
        <span id={entriesErrorId} className="db-cfg-warn" role="alert">
          {entriesError}
        </span>
      ) : null}
      {bytesError ? (
        <span id={bytesErrorId} className="db-cfg-warn" role="alert">
          {bytesError}
        </span>
      ) : null}
    </SettingsSection>
  )
}

export function DatabaseConfigForm(props: ConfigFormProps & { host: Host }) {
  const value = asObject(props.value)
  const databases = asObject(value.databases)
  const names = Object.keys(databases)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const activeIndex = names.length > 0 ? Math.min(selectedIndex, names.length - 1) : -1
  const activeName = activeIndex >= 0 ? names[activeIndex] : undefined
  const [responsiveRootRef, narrow] = useContainerNarrow(CONFIG_NARROW_BELOW)

  // Probe outcomes are keyed by handle and dropped on any edit of that
  // handle — a stale "connected" next to a changed url would be a lie. The
  // token guards the async completion: an edit/rename/remove while a probe
  // is in flight bumps it, and the completion for the superseded probe is
  // discarded instead of resurrecting a result for a url it never tested.
  const [testResults, setTestResults] = useState<Record<string, TestResult>>({})
  const testTokens = useRef<Record<string, number>>({})

  const commit = (nextDatabases: JsonObject) => props.onChange({ ...value, databases: nextDatabases })

  const clearTest = (name: string) => {
    testTokens.current[name] = (testTokens.current[name] ?? 0) + 1
    setTestResults((r) => {
      const next = { ...r }
      delete next[name]
      return next
    })
  }

  const setDb = (name: string, next: JsonObject) => {
    clearTest(name)
    commit({ ...databases, [name]: next })
  }

  const runTest = async (name: string) => {
    const db = asObject(databases[name])
    const token = (testTokens.current[name] = (testTokens.current[name] ?? 0) + 1)
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
          ? `Connected · ${resp.server_version ?? resp.driver} · ${resp.latency_ms}ms`
          : (resp.message ?? 'Connection failed'),
      }
    } catch (e) {
      result = { status: 'done', ok: false, text: errText(e) }
    }
    if (testTokens.current[name] !== token) return // superseded by an edit
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
    setSelectedIndex(names.length)
    commit({ ...databases, [name]: { url: `sqlite:./data/${name}.db` } })
  }

  const renameDb = (from: string, to: string) => {
    if (to === from || databases[to] !== undefined) return
    clearTest(from)
    // Rebuild in place so the card doesn't jump to the end of the list.
    const next: JsonObject = {}
    for (const key of names) {
      next[key === from ? to : key] = databases[key]
    }
    commit(next)
  }

  // The host passes the deep-link path to the custom form. Prefer the exact
  // control, then fall back to the named database card.
  const rootRef = useRef<HTMLDivElement | null>(null)
  const setRootRef = useCallback(
    (node: HTMLDivElement | null) => {
      rootRef.current = node
      responsiveRootRef(node)
    },
    [responsiveRootRef],
  )
  const handledFocusKeyRef = useRef('')
  const detailFocusFrameRef = useRef<number | null>(null)
  const focusRequest = databaseFocusRequest(names, props.focusField)
  const focusRequestKey = focusRequest?.key ?? ''
  const focusExactField = focusRequest?.exactField ?? ''
  const focusDatabaseIndex = focusRequest?.databaseIndex ?? -1
  const focusDatabaseName = focusRequest?.databaseName ?? ''

  const selectDatabase = (index: number) => {
    setSelectedIndex(index)
    if (!narrow) return
    if (detailFocusFrameRef.current !== null) cancelAnimationFrame(detailFocusFrameRef.current)
    detailFocusFrameRef.current = requestAnimationFrame(() => {
      detailFocusFrameRef.current = null
      focusDatabaseDetail(rootRef.current)
    })
  }

  useEffect(
    () => () => {
      if (detailFocusFrameRef.current !== null) cancelAnimationFrame(detailFocusFrameRef.current)
    },
    [],
  )

  useEffect(() => {
    if (selectedIndex < names.length || names.length === 0) return
    setSelectedIndex(names.length - 1)
  }, [names.length, selectedIndex])

  useEffect(() => {
    if (!focusRequestKey) {
      handledFocusKeyRef.current = ''
      return
    }
    if (handledFocusKeyRef.current === focusRequestKey || !rootRef.current) return
    if (focusDatabaseIndex >= 0 && focusDatabaseIndex !== activeIndex) {
      setSelectedIndex(focusDatabaseIndex)
      return
    }

    const target =
      rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusExactField)}"]`) ??
      (focusDatabaseName
        ? rootRef.current.querySelector<HTMLElement>(`[data-field="db-${CSS.escape(focusDatabaseName)}"]`)
        : undefined)
    handledFocusKeyRef.current = focusRequestKey
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [activeIndex, focusDatabaseIndex, focusDatabaseName, focusExactField, focusRequestKey])

  return (
    <div className="db-cfg" ref={setRootRef}>
      <HistorySettings value={value} errors={props.errors} onChange={props.onChange} />

      <SettingsSection
        className="db-cfg-connections"
        title="Connections"
        description="Each handle identifies a connection pool used by database functions."
        action={
          names.length > 0 ? (
            <button type="button" className="db-cfg-add" onClick={addDb}>
              Add database
            </button>
          ) : undefined
        }
      >
        {names.length === 0 ? (
          <div className="db-cfg-empty">
            <strong>No databases configured</strong>
            <p>The worker needs at least one connection before it can start.</p>
            <button type="button" className="db-cfg-add" onClick={addDb}>
              Add database
            </button>
          </div>
        ) : (
          <div className="db-cfg-master-detail">
            <aside className="db-cfg-master" aria-label="Configured databases">
              <SettingsList className="db-cfg-connection-list">
                {names.map((name, index) => {
                  const db = asObject(databases[name])
                  const driver = driverOfConfiguredUrl(asString(db.url))
                  const capture = asString(db.capture) || 'statements'
                  const active = index === activeIndex
                  return (
                    <SettingsRow
                      // biome-ignore lint/suspicious/noArrayIndexKey: a stable row keeps its identity while the editable handle is renamed.
                      key={index}
                      className={`db-cfg-connection-row${active ? ' is-active' : ''}`}
                      layout="inline"
                      label={
                        <button
                          type="button"
                          className="db-cfg-connection-select"
                          aria-current={active ? 'page' : undefined}
                          onClick={() => selectDatabase(index)}
                        >
                          {name}
                        </button>
                      }
                      description={capture === 'native' ? 'Native row capture' : 'Worker statement capture'}
                      control={
                        <span className={`db-cfg-driver db-cfg-driver-${driver}`}>
                          {driver === 'unknown' && asString(db.url).includes('${')
                            ? 'From environment'
                            : DRIVER_LABELS[driver]}
                        </span>
                      }
                    />
                  )
                })}
              </SettingsList>
            </aside>

            {activeName !== undefined ? (
              <DatabaseDetail
                key={activeIndex}
                name={activeName}
                db={asObject(databases[activeName])}
                onRename={(to) => renameDb(activeName, to)}
                onChange={(next) => setDb(activeName, next)}
                onRemove={() => {
                  removeDb(activeName)
                  setSelectedIndex(Math.min(activeIndex, names.length - 2))
                }}
                removable={names.length > 1}
                test={testResults[activeName]}
                onTest={() => runTest(activeName)}
                errors={props.errors}
              />
            ) : null}
          </div>
        )}
      </SettingsSection>

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

function DatabaseDetail(card: {
  name: string
  db: JsonObject
  onRename: (to: string) => void
  onChange: (next: JsonObject) => void
  onRemove: () => void
  removable: boolean
  test: TestResult | undefined
  onTest: () => void
  errors: ConfigFormProps['errors']
}) {
  const { name, db } = card
  const url = asString(db.url)
  const driver = driverOfConfiguredUrl(url)
  const environmentUrl = url.includes('${')
  const showTls = shouldShowTlsForUrl(url)
  const maskUrl = shouldMaskConfiguredUrl(url)
  const captureValue = db.capture
  const capture = asString(captureValue) || 'statements'
  const tls = asObject(db.tls)
  const pool = asObject(db.pool)
  const trustNativeRaw = isRawTypedValue(tls.trust_native) ? tls.trust_native : undefined
  const isMemorySqlite = driver === 'sqlite' && url.includes(':memory:')
  const [showUrl, setShowUrl] = useState(false)
  const urlError = fieldError(card.errors, ['databases', name, 'url'])
  const captureError = fieldError(card.errors, ['databases', name, 'capture'])
  const tlsModeError = fieldError(card.errors, ['databases', name, 'tls', 'mode'])
  const tlsCaError = fieldError(card.errors, ['databases', name, 'tls', 'ca_cert'])
  const tlsTrustError = fieldError(card.errors, ['databases', name, 'tls', 'trust_native'])
  const fieldId = (field: string) => `db-cfg-${field}-${name}`
  const errorId = (field: string, error: string | undefined) => (error ? `${fieldId(field)}-error` : undefined)

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
    <div className="db-cfg-detail" data-field={`db-${name}`} tabIndex={-1}>
      <SettingsSection
        className="db-cfg-detail-section"
        title={name}
        description="Connection identity, driver, and endpoint."
        action={
          card.removable ? (
            <button
              type="button"
              className="db-cfg-remove"
              aria-label={`Remove database ${name}`}
              onClick={card.onRemove}
            >
              Remove
            </button>
          ) : undefined
        }
      >
        <SettingsList>
          <SettingsRow
            label={<label htmlFor={fieldId('handle')}>Handle</label>}
            description="Used as the database argument in worker functions."
            control={
              <input
                id={fieldId('handle')}
                name={`databases.${name}.handle`}
                className="db-cfg-input db-cfg-name"
                type="text"
                value={name}
                spellCheck={false}
                autoComplete="off"
                onChange={(event) => card.onRename(event.target.value)}
                onBlur={(event) => card.onRename(event.target.value.trim())}
              />
            }
          />
          <SettingsRow
            label="Driver"
            description="Inferred from the connection URL and not stored separately."
            control={
              <output
                className={`db-cfg-driver db-cfg-driver-${driver}`}
                data-field={`databases.${name}.driver`}
                htmlFor={fieldId('url')}
              >
                {driver === 'unknown' && environmentUrl ? 'From environment' : DRIVER_LABELS[driver]}
              </output>
            }
          />
          <SettingsRow
            className="db-cfg-url-setting"
            layout="stacked"
            label={<label htmlFor={fieldId('url')}>Connection URL</label>}
            description="PostgreSQL, MySQL, and SQLite URLs are supported. Credentials stay masked for network URLs."
            meta={
              card.test?.status === 'done' || urlError ? (
                <>
                  {card.test?.status === 'done' ? (
                    <span className={card.test.ok ? 'db-cfg-test-ok' : 'db-cfg-test-fail'} role="status">
                      {card.test.text}
                    </span>
                  ) : null}
                  {urlError ? (
                    <span id={errorId('url', urlError)} className="db-cfg-warn" role="alert">
                      {urlError}
                    </span>
                  ) : null}
                </>
              ) : undefined
            }
            control={
              <div className="db-cfg-url-row">
                <input
                  id={fieldId('url')}
                  name={`databases.${name}.url`}
                  data-field={`databases.${name}.url`}
                  className="db-cfg-input db-cfg-grow"
                  type={!maskUrl || showUrl ? 'text' : 'password'}
                  value={url}
                  placeholder="postgres://user:pass@host:5432/db · mysql://… · sqlite:./data/app.db"
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={urlError ? true : undefined}
                  aria-describedby={errorId('url', urlError)}
                  onChange={(event) => set((next) => (next.url = event.target.value))}
                />
                {maskUrl ? (
                  <button
                    type="button"
                    className="db-cfg-reveal"
                    aria-label={showUrl ? 'Hide connection URL' : 'Show connection URL'}
                    aria-pressed={showUrl}
                    onClick={() => setShowUrl((current) => !current)}
                  >
                    {showUrl ? 'Hide' : 'Show'}
                  </button>
                ) : null}
                <button
                  type="button"
                  className="db-cfg-test"
                  disabled={card.test?.status === 'testing' || url.trim() === ''}
                  onClick={card.onTest}
                >
                  {card.test?.status === 'testing' ? 'Testing…' : 'Test connection'}
                </button>
              </div>
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        className="db-cfg-detail-section"
        title="Change capture"
        description="Choose which writes can emit database::row-changed events."
      >
        <SettingsList>
          <SettingsRow
            label={<label htmlFor={`db-cfg-capture-${name}`}>Row-change capture</label>}
            description={CAPTURE_HINTS[driver]}
            meta={
              captureError || (capture === 'native' && isMemorySqlite) ? (
                <>
                  {captureError ? (
                    <span id={errorId('capture', captureError)} className="db-cfg-warn" role="alert">
                      {captureError}
                    </span>
                  ) : null}
                  {capture === 'native' && isMemorySqlite ? (
                    <span className="db-cfg-warn" role="alert">
                      A `:memory:` database is per-connection and cannot be captured.
                    </span>
                  ) : null}
                </>
              ) : undefined
            }
            control={
              <DatabaseSelectInput
                id={`db-cfg-capture-${name}`}
                name={`databases.${name}.capture`}
                dataField={`databases.${name}.capture`}
                label="Row-change capture"
                value={captureValue}
                fallback="statements"
                options={[
                  {
                    value: 'statements',
                    label: 'Statements — writes through this worker',
                  },
                  {
                    value: 'native',
                    label: 'Native — writes from any client',
                  },
                ]}
                error={captureError}
                errorId={errorId('capture', captureError)}
                onChange={(nextValue) =>
                  set((next) => {
                    if (nextValue === 'statements') delete next.capture
                    else next.capture = nextValue
                  })
                }
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      {showTls ? (
        <SettingsSection
          className="db-cfg-detail-section"
          title="Transport security"
          description="TLS settings remain available for network and environment-provided connection URLs."
        >
          <SettingsList>
            <SettingsRow
              label={<label htmlFor={`db-cfg-tls-${name}`}>TLS mode</label>}
              description="Control encryption and hostname verification for network connections."
              meta={
                tlsModeError ? (
                  <span id={errorId('tls-mode', tlsModeError)} className="db-cfg-warn" role="alert">
                    {tlsModeError}
                  </span>
                ) : undefined
              }
              control={
                <DatabaseSelectInput
                  id={`db-cfg-tls-${name}`}
                  name={`databases.${name}.tls.mode`}
                  dataField={`databases.${name}.tls.mode`}
                  label="TLS mode"
                  value={tls.mode}
                  fallback="require"
                  options={[
                    { value: 'disable', label: 'Disable — plaintext' },
                    { value: 'require', label: 'Require — validate chain' },
                    { value: 'verify-full', label: 'Verify full — validate hostname' },
                  ]}
                  error={tlsModeError}
                  errorId={errorId('tls-mode', tlsModeError)}
                  onChange={(nextValue) =>
                    setBlock('tls', (block) => {
                      if (nextValue === 'require') delete block.mode
                      else block.mode = nextValue
                    })
                  }
                />
              }
            />
            <SettingsRow
              label={<label htmlFor={`db-cfg-ca-${name}`}>Extra CA bundle</label>}
              description="Optional path to a PEM bundle for private certificate authorities."
              meta={
                tlsCaError ? (
                  <span id={errorId('tls-ca', tlsCaError)} className="db-cfg-warn" role="alert">
                    {tlsCaError}
                  </span>
                ) : undefined
              }
              control={
                <input
                  id={`db-cfg-ca-${name}`}
                  name={`databases.${name}.tls.ca_cert`}
                  data-field={`databases.${name}.tls.ca_cert`}
                  className="db-cfg-input"
                  type="text"
                  value={asString(tls.ca_cert)}
                  placeholder="/etc/ssl/private-ca.pem"
                  spellCheck={false}
                  autoComplete="off"
                  aria-invalid={tlsCaError ? true : undefined}
                  aria-describedby={errorId('tls-ca', tlsCaError)}
                  onChange={(event) =>
                    setBlock('tls', (block) => {
                      if (event.target.value === '') delete block.ca_cert
                      else block.ca_cert = event.target.value
                    })
                  }
                />
              }
            />
            <SettingsRow
              className="db-cfg-trust-row"
              label="Use the system trust store"
              description={
                driver === 'mysql'
                  ? 'MySQL always includes its bundled public roots. This setting only changes PostgreSQL behavior.'
                  : 'Combine native system roots with the optional CA bundle. Turn off to trust only the supplied bundle.'
              }
              meta={
                tlsTrustError ? (
                  <span id={errorId('tls-trust', tlsTrustError)} className="db-cfg-warn" role="alert">
                    {tlsTrustError}
                  </span>
                ) : undefined
              }
              control={
                trustNativeRaw !== undefined ? (
                  <RawTypedValue
                    id={`db-cfg-trust-native-${name}`}
                    name={`databases.${name}.tls.trust_native`}
                    dataField={`databases.${name}.tls.trust_native`}
                    label="Use the system trust store"
                    value={trustNativeRaw}
                    replacementLabel={booleanLiteralForRawValue(trustNativeRaw, true) ? 'on' : 'off'}
                    error={tlsTrustError}
                    errorId={errorId('tls-trust', tlsTrustError)}
                    onChange={(raw) =>
                      setBlock('tls', (block) => {
                        block.trust_native = raw
                      })
                    }
                    onUseLiteral={() =>
                      setBlock('tls', (block) => {
                        if (booleanLiteralForRawValue(trustNativeRaw, true)) delete block.trust_native
                        else block.trust_native = false
                      })
                    }
                  />
                ) : (
                  <Switch
                    name={`databases.${name}.tls.trust_native`}
                    data-field={`databases.${name}.tls.trust_native`}
                    aria-label="Use the system trust store"
                    aria-invalid={tlsTrustError ? true : undefined}
                    aria-describedby={errorId('tls-trust', tlsTrustError)}
                    checked={asBoolean(tls.trust_native, true)}
                    onChange={(event) =>
                      setBlock('tls', (block) => {
                        if (event.target.checked) delete block.trust_native
                        else block.trust_native = false
                      })
                    }
                  />
                )
              }
            />
          </SettingsList>
          {driver === 'postgres' && !asBoolean(tls.trust_native, true) && asString(tls.ca_cert).trim() === '' ? (
            <div className="db-cfg-notice" role="alert">
              Add a CA bundle before disabling the system trust store, or the PostgreSQL pool cannot establish trust.
            </div>
          ) : null}
        </SettingsSection>
      ) : null}

      <SettingsSection
        className="db-cfg-detail-section"
        title="Connection pool"
        description="Bound concurrency and how long callers wait for an available connection."
      >
        <SettingsList>
          {POOL_FIELDS.map((field) => {
            const error = fieldError(card.errors, ['databases', name, 'pool', field.key])
            return (
              <SettingsRow
                key={field.key}
                label={<label htmlFor={`db-cfg-${field.key}-${name}`}>{field.label}</label>}
                description={field.description}
                meta={
                  error ? (
                    <span id={errorId(`pool-${field.key}`, error)} className="db-cfg-warn" role="alert">
                      {error}
                    </span>
                  ) : (
                    `${Number(field.placeholder).toLocaleString('en-US')} by default`
                  )
                }
                control={
                  <DatabaseNumberInput
                    id={`db-cfg-${field.key}-${name}`}
                    name={`databases.${name}.pool.${field.key}`}
                    dataField={`databases.${name}.pool.${field.key}`}
                    label={field.label}
                    value={pool[field.key]}
                    fallback={Number(field.placeholder)}
                    error={error}
                    errorId={errorId(`pool-${field.key}`, error)}
                    onChange={(raw) =>
                      setBlock('pool', (block) => {
                        if (raw.trim() === '') delete block[field.key]
                        else {
                          const parsed = Number(raw)
                          block[field.key] = Number.isInteger(parsed) && parsed >= 0 ? parsed : raw
                        }
                      })
                    }
                  />
                }
              />
            )
          })}
        </SettingsList>
      </SettingsSection>
    </div>
  )
}
